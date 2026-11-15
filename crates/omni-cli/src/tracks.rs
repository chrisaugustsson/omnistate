//! `omni track`: create, list, show, rename and delete the tracks of a
//! project (Q51). A track is a part of a project with docs and threads of
//! its own. It has no status (V7); a delete is soft and takes its
//! docs and threads with it.

use crate::args::Args;
use crate::commands::{Agent, agent, say};
use crate::error::{CliError, Result, bail};
use crate::json::{Value, quote};
use crate::projects::{is_uuid, project_id, projects_path, resolve_track};

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

fn tracks_path(agent: &Agent, project_id: &str) -> String {
    format!("{}/{project_id}/tracks", projects_path(agent))
}

/// One line: ID and name.
fn summary(track: &Value) -> String {
    let field = |name: &str| track.str_at(name).unwrap_or("?");
    format!("{}  {}", field("id"), field("name"))
}

pub fn create(args: &Args, name: Option<&str>) -> Result<()> {
    let name = name.map(str::trim).unwrap_or_default();
    if name.is_empty() {
        bail!("a track needs a name: omni track create <name> [--description <text>]");
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
        &tracks_path(&agent, &project),
        Some(body.as_bytes()),
    )?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let body = parsed(&text, "the new track")?;
    let track = body.get("track").unwrap_or(&body);
    say(&format!(
        "{}  {}",
        track.str_at("id").unwrap_or("?"),
        track.str_at("name").unwrap_or(name)
    ));
    Ok(())
}

pub fn list(args: &Args) -> Result<()> {
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let response = agent.request("GET", &tracks_path(&agent, &project), None)?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let body = parsed(&text, "the tracks")?;
    let tracks = body
        .get("tracks")
        .and_then(Value::as_array)
        .unwrap_or_default();
    if tracks.is_empty() {
        say("no tracks");
    }
    for track in tracks {
        say(&summary(track));
    }
    Ok(())
}

pub fn show(args: &Args, track: Option<&str>) -> Result<()> {
    let Some(given) = track else {
        bail!("`omni track show` needs a track name or ID; `omni track list` shows them");
    };
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let path = tracks_path(&agent, &project);
    let text = if is_uuid(given) {
        let response = agent.request(
            "GET",
            &format!("{path}/{}", given.to_ascii_lowercase()),
            None,
        )?;
        response.text()
    } else {
        let response = agent.request("GET", &path, None)?;
        let listed = parsed(&response.text(), "the tracks")?;
        let found = listed
            .get("tracks")
            .and_then(Value::as_array)
            .unwrap_or_default()
            .iter()
            .find(|track| {
                track
                    .str_at("name")
                    .is_some_and(|name| name.eq_ignore_ascii_case(given.trim()))
            })
            .ok_or_else(|| {
                CliError::new(format!(
                    "no track named {given:?}; `omni track list` shows the tracks"
                ))
            })?;
        format!("{{\"track\":{found}}}")
    };
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let body = parsed(&text, "the track")?;
    let track = body.get("track").unwrap_or(&body);
    say(&summary(track));
    say(&format!(
        "project: {}",
        track.str_at("project_id").unwrap_or("?")
    ));
    if let Some(description) = track.str_at("description").filter(|d| !d.is_empty()) {
        say("description:");
        for line in description.lines() {
            say(&format!("  {line}"));
        }
    }
    Ok(())
}

/// `omni track rename <name|id> <new name>`
pub fn rename(args: &Args, track: Option<&str>, name: Option<&str>) -> Result<()> {
    let usage = "omni track rename <name|id> <new name>";
    let Some(given) = track else {
        bail!("a track rename needs the track and its new name: {usage}");
    };
    let name = name.map(str::trim).unwrap_or_default();
    if name.is_empty() {
        bail!("a track rename needs a new name: {usage}");
    }
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let id = resolve_track(&agent, &project, given)?;
    let response = agent.request(
        "PATCH",
        &format!("{}/{id}", tracks_path(&agent, &project)),
        Some(format!("{{\"name\":{}}}", quote(name)).as_bytes()),
    )?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let body = parsed(&text, "the renamed track")?;
    let track = body.get("track").unwrap_or(&body);
    say(&format!(
        "{}  {}",
        track.str_at("id").unwrap_or(&id),
        track.str_at("name").unwrap_or(name)
    ));
    Ok(())
}

/// `omni track delete <name|id>`
pub fn delete(args: &Args, track: Option<&str>) -> Result<()> {
    let Some(given) = track else {
        bail!("`omni track delete` needs a track name or ID: omni track delete <name|id>");
    };
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let id = resolve_track(&agent, &project, given)?;
    agent.request(
        "DELETE",
        &format!("{}/{id}", tracks_path(&agent, &project)),
        None,
    )?;
    if args.json {
        say(&format!("{{\"deleted\":{}}}", quote(&id)));
    } else {
        say(&format!("deleted track {id}"));
    }
    Ok(())
}
