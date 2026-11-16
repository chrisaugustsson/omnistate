//! `omni track`: create, list, show, rename and delete the tracks of a
//! project (Q51). A track is a part of a project with docs and threads of
//! its own, and a state (planning, building, done or dropped); a delete is
//! soft and takes its docs and threads with it.

use crate::args::Args;
use crate::commands::{Agent, agent, say};
use crate::error::{CliError, Result, bail};
use crate::features::{Found, resolve_feature};
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

/// The parts of the project that are not deleted.
fn project_parts(agent: &Agent, project_id: &str) -> Result<Vec<Value>> {
    let path = format!("{}/{project_id}/parts", projects_path(agent));
    let listed = parsed(&agent.request("GET", &path, None)?.text(), "the parts")?;
    Ok(listed
        .get("parts")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .to_vec())
}

/// The ID of the part `given` names: an ID as it is (the API says whether it
/// is a part of the project), a name among the parts that are not deleted,
/// ignoring case.
fn resolve_part(agent: &Agent, project_id: &str, given: &str) -> Result<String> {
    if is_uuid(given) {
        return Ok(given.to_ascii_lowercase());
    }
    project_parts(agent, project_id)?
        .iter()
        .find(|part| {
            part.str_at("name")
                .is_some_and(|name| name.eq_ignore_ascii_case(given.trim()))
        })
        .and_then(|part| part.str_at("id"))
        .map(str::to_string)
        .ok_or_else(|| {
            CliError::new(format!(
                "unknown_part: no part named {given:?}; `omni part list` shows the parts"
            ))
        })
}

/// One line: ID, name, the part's name (its ID when the part is not listed),
/// the state and the owner's user ID (`-` when no member owns the track).
fn summary(track: &Value, parts: &[Value]) -> String {
    let field = |name: &str| track.str_at(name).unwrap_or("?");
    let part_id = field("part_id");
    let part = parts
        .iter()
        .find(|part| part.str_at("id") == Some(part_id))
        .and_then(|part| part.str_at("name"))
        .unwrap_or(part_id);
    format!(
        "{}  {}  {part}  {}  {}",
        field("id"),
        field("name"),
        field("state"),
        track.str_at("owner_user_id").unwrap_or("-")
    )
}

pub fn create(args: &Args, name: Option<&str>) -> Result<()> {
    let name = name.map(str::trim).unwrap_or_default();
    if name.is_empty() {
        bail!("a track needs a name: omni track create <name> --part <name|id>");
    }
    let Some(part) = &args.part else {
        bail!("part_required: a track sits on a part: omni track create <name> --part <name|id>");
    };
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let part_id = resolve_part(&agent, &project, part)?;
    let mut body = format!("{{\"name\":{},\"part_id\":{}", quote(name), quote(&part_id));
    if let Some(description) = &args.description {
        body.push_str(&format!(",\"description\":{}", quote(description)));
    }
    body.push('}');
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

/// `omni track split <name> --part <name|id> --from <track>`: a new track on
/// the part, owned by the caller, linked to the track it is split off from.
pub fn split(args: &Args, name: Option<&str>) -> Result<()> {
    let usage = "omni track split <name> --part <name|id> --from <track>";
    let name = name.map(str::trim).unwrap_or_default();
    if name.is_empty() {
        bail!("a track split needs a name: {usage}");
    }
    let (Some(part), Some(from)) = (&args.part, &args.from) else {
        bail!("a track split needs --part and --from: {usage}");
    };
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let part_id = resolve_part(&agent, &project, part)?;
    let from_id = resolve_track(&agent, &project, from)?;
    let mut body = format!(
        "{{\"name\":{},\"part_id\":{},\"split_from_track_id\":{}",
        quote(name),
        quote(&part_id),
        quote(&from_id)
    );
    if let Some(description) = &args.description {
        body.push_str(&format!(",\"description\":{}", quote(description)));
    }
    body.push('}');
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
    say(&summary(track, &project_parts(&agent, &project)?));
    Ok(())
}

pub fn list(args: &Args) -> Result<()> {
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let part_id = match &args.part {
        Some(part) => Some(resolve_part(&agent, &project, part)?),
        None => None,
    };
    let mut path = tracks_path(&agent, &project);
    if args.mine {
        path.push_str("?owner=me");
    }
    let response = agent.request("GET", &path, None)?;
    let text = response.text();
    if args.json && part_id.is_none() {
        print_json(&text);
        return Ok(());
    }
    let body = parsed(&text, "the tracks")?;
    let tracks: Vec<&Value> = body
        .get("tracks")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter(|track| {
            part_id
                .as_deref()
                .is_none_or(|id| track.str_at("part_id") == Some(id))
        })
        .collect();
    if args.json {
        let items: Vec<String> = tracks.iter().map(|track| track.to_string()).collect();
        say(&format!("{{\"tracks\":[{}]}}", items.join(",")));
        return Ok(());
    }
    if tracks.is_empty() {
        say("no tracks");
    }
    let parts = project_parts(&agent, &project)?;
    for track in tracks {
        say(&summary(track, &parts));
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
    say(&summary(track, &project_parts(&agent, &project)?));
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
    let id = track.str_at("id").unwrap_or(given);
    let all = parsed(
        &agent.request("GET", &path, None)?.text(),
        "the project's tracks",
    )?;
    let all = all
        .get("tracks")
        .and_then(Value::as_array)
        .unwrap_or_default();
    let line = |other: &Value| {
        format!(
            "  {}  {}  {}",
            other.str_at("id").unwrap_or("?"),
            other.str_at("name").unwrap_or("?"),
            other.str_at("state").unwrap_or("?")
        )
    };
    if let Some(from) = track.str_at("split_from_track_id") {
        say("split off from:");
        match all.iter().find(|other| other.str_at("id") == Some(from)) {
            Some(other) => say(&line(other)),
            None => say(&format!("  {from}")),
        }
    }
    let off_to: Vec<&Value> = all
        .iter()
        .filter(|other| other.str_at("split_from_track_id") == Some(id))
        .collect();
    if !off_to.is_empty() {
        say("split off to:");
        for other in off_to {
            say(&line(other));
        }
    }
    let features = parsed(
        &agent
            .request("GET", &format!("{path}/{id}/features"), None)?
            .text(),
        "the track's features",
    )?;
    let features = features
        .get("features")
        .and_then(Value::as_array)
        .unwrap_or_default();
    if !features.is_empty() {
        say("features:");
        for feature in features {
            say(&format!(
                "  {}  {}",
                feature.str_at("id").unwrap_or("?"),
                feature.str_at("name").unwrap_or("?")
            ));
        }
    }
    Ok(())
}

/// The feature `given` names for a track: among the features of the track's
/// own part first, then anywhere in the project (so that a name only on
/// another part reaches the API, which refuses it).
fn resolve_track_feature(
    agent: &Agent,
    project: &str,
    track_id: &str,
    given: &str,
) -> Result<Found> {
    let response = agent.request(
        "GET",
        &format!("{}/{track_id}", tracks_path(agent, project)),
        None,
    )?;
    let body = parsed(&response.text(), "the track")?;
    let track = body.get("track").unwrap_or(&body);
    if let Some(part_id) = track.str_at("part_id")
        && let Ok(found) = resolve_feature(agent, project, given, Some(part_id))
    {
        return Ok(found);
    }
    resolve_feature(agent, project, given, None)
}

/// `omni track link-feature <track> <feature>` and
/// `omni track unlink-feature <track> <feature>`
pub fn link_feature(
    args: &Args,
    track: Option<&str>,
    feature: Option<&str>,
    unlink: bool,
) -> Result<()> {
    let usage = if unlink {
        "omni track unlink-feature <track> <feature>"
    } else {
        "omni track link-feature <track> <feature>"
    };
    let (Some(given), Some(feature)) = (track, feature) else {
        bail!("a track needs its track and its feature: {usage}");
    };
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let id = resolve_track(&agent, &project, given)?;
    let found = resolve_track_feature(&agent, &project, &id, feature)?;
    let path = format!("{}/{id}/features", tracks_path(&agent, &project));
    if unlink {
        agent.request("DELETE", &format!("{path}/{}", found.id()), None)?;
        if args.json {
            say(&format!("{{\"unlinked\":{}}}", quote(found.id())));
        } else {
            say(&format!("unlinked feature {}", found.id()));
        }
        return Ok(());
    }
    let response = agent.request(
        "POST",
        &path,
        Some(format!("{{\"feature_id\":{}}}", quote(found.id())).as_bytes()),
    )?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let body = parsed(&text, "the linked feature")?;
    let linked = body.get("feature").unwrap_or(&body);
    say(&format!(
        "{}  {}",
        linked.str_at("id").unwrap_or(found.id()),
        linked.str_at("name").unwrap_or(feature)
    ));
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

/// `omni track move <name|id> --part <name|id>`
pub fn move_to(args: &Args, track: Option<&str>) -> Result<()> {
    let usage = "omni track move <name|id> --part <name|id>";
    let Some(given) = track else {
        bail!("a track move needs the track and its part: {usage}");
    };
    let Some(part) = &args.part else {
        bail!("part_required: a track move needs a part: {usage}");
    };
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let id = resolve_track(&agent, &project, given)?;
    let part_id = resolve_part(&agent, &project, part)?;
    let response = agent.request(
        "POST",
        &format!("{}/{id}/move", tracks_path(&agent, &project)),
        Some(format!("{{\"part_id\":{}}}", quote(&part_id)).as_bytes()),
    )?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let body = parsed(&text, "the moved track")?;
    let track = body.get("track").unwrap_or(&body);
    say(&summary(track, &project_parts(&agent, &project)?));
    Ok(())
}

/// `omni track state <name|id> planning|building|done|dropped`
pub fn state(args: &Args, track: Option<&str>, to: Option<&str>) -> Result<()> {
    let usage = "omni track state <name|id> planning|building|done|dropped";
    let (Some(given), Some(to)) = (track, to) else {
        bail!("a track state needs the track and its new state: {usage}");
    };
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let id = resolve_track(&agent, &project, given)?;
    let response = agent.request(
        "POST",
        &format!("{}/{id}/state", tracks_path(&agent, &project)),
        Some(format!("{{\"state\":{}}}", quote(to.trim())).as_bytes()),
    )?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let body = parsed(&text, "the track")?;
    let track = body.get("track").unwrap_or(&body);
    say(&summary(track, &project_parts(&agent, &project)?));
    Ok(())
}

/// `omni track handover <name|id> <user-id>`: the user's membership in the
/// workspace gets the track. A user that is no member fails before any call
/// to the track.
pub fn handover(args: &Args, track: Option<&str>, user: Option<&str>) -> Result<()> {
    let usage = "omni track handover <name|id> <user-id>";
    let (Some(given), Some(user)) = (track, user) else {
        bail!("a track handover needs the track and the user: {usage}");
    };
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let id = resolve_track(&agent, &project, given)?;
    let path = format!("/api/v1/w/{}/members", agent.workspace_id);
    let listed = parsed(&agent.request("GET", &path, None)?.text(), "the members")?;
    let membership = listed
        .get("members")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .find(|member| {
            member
                .str_at("user_id")
                .is_some_and(|id| id.eq_ignore_ascii_case(user.trim()))
        })
        .and_then(|member| member.str_at("membership_id"))
        .ok_or_else(|| {
            CliError::new(format!(
                "not_a_member: {user} is not a member of this workspace"
            ))
        })?;
    let response = agent.request(
        "POST",
        &format!("{}/{id}/owner", tracks_path(&agent, &project)),
        Some(format!("{{\"membership_id\":{}}}", quote(membership)).as_bytes()),
    )?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let body = parsed(&text, "the track")?;
    let track = body.get("track").unwrap_or(&body);
    say(&summary(track, &project_parts(&agent, &project)?));
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
