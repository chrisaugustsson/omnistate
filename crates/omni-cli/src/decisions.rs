//! `omni decision`: record a decision on a track and list the decisions of a
//! track, a feature, a part or the whole project. A decision says what was
//! decided and why, may be about one feature, one part or the whole project,
//! and may replace an earlier decision; it is never edited or deleted.

use crate::args::Args;
use crate::commands::{Agent, agent, say};
use crate::error::{CliError, Result, bail};
use crate::features::resolve_feature;
use crate::json::{Value, quote};
use crate::parts::resolve_part;
use crate::projects::{current_track, is_uuid, project_id, projects_path, resolve_track};

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
    format!("{}/{project_id}", projects_path(agent))
}

/// The text of a required option, trimmed.
fn required<'a>(value: &'a Option<String>, flag: &str, usage: &str) -> Result<&'a str> {
    match value.as_deref().map(str::trim) {
        Some(text) if !text.is_empty() => Ok(text),
        _ => bail!("a decision needs {flag}: {usage}"),
    }
}

/// Fails with `one_scope_only` when more than one level is given.
fn one_level(args: &Args) -> Result<()> {
    let given = [
        args.feature.is_some(),
        args.part.is_some(),
        args.project_level,
    ];
    if given.into_iter().filter(|set| *set).count() > 1 {
        bail!("one_scope_only: give one of --feature, --part and --project, not several");
    }
    Ok(())
}

/// `omni decision add --question <q> --decision <d> --why <w> [--feature <f> | --part <p> | --project] [--replaces <id>]`
pub fn add(args: &Args) -> Result<()> {
    const USAGE: &str = "omni decision add --question <q> --decision <d> --why <w> [--feature <name|id> | --part <name|id> | --project] [--replaces <id>]";
    one_level(args)?;
    let question = required(&args.question, "--question", USAGE)?;
    let decision = required(&args.decision, "--decision", USAGE)?;
    let why = required(&args.why, "--why", USAGE)?;
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let Some(track) = current_track(args) else {
        bail!("a decision belongs to a track: pass --track <name|id> or set OMNI_TRACK");
    };
    let track = resolve_track(&agent, &project, &track)?;
    let mut body = format!(
        "{{\"question\":{},\"decision\":{},\"why\":{}",
        quote(question),
        quote(decision),
        quote(why)
    );
    if let Some(given) = args.feature.as_deref() {
        // An ID goes to the API as it is, which says whose project it is of.
        let feature = if is_uuid(given.trim()) {
            given.trim().to_ascii_lowercase()
        } else {
            resolve_feature(&agent, &project, given, None)?
                .id()
                .to_string()
        };
        body.push_str(&format!(",\"feature_id\":{}", quote(&feature)));
    }
    if let Some(given) = args.part.as_deref() {
        let part = resolve_part(&agent, &project, given.trim())?;
        body.push_str(&format!(",\"part_id\":{}", quote(&part)));
    }
    if args.project_level {
        body.push_str(",\"about_project\":true");
    }
    if let Some(replaces) = args.replaces.as_deref() {
        body.push_str(&format!(
            ",\"replaces_decision_id\":{}",
            quote(replaces.trim())
        ));
    }
    body.push('}');
    let path = format!(
        "{}/tracks/{track}/decisions",
        project_path(&agent, &project)
    );
    let response = agent.request("POST", &path, Some(body.as_bytes()))?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let answer = parsed(&text, "the new decision")?;
    let recorded = answer.get("decision").unwrap_or(&answer);
    say(&format!(
        "recorded decision {}",
        recorded.str_at("id").unwrap_or("?")
    ));
    Ok(())
}

/// `omni decision list [--track <name|id> | --feature <name|id> | --part <name|id> | --project]`
pub fn list(args: &Args) -> Result<()> {
    one_level(args)?;
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let path = if args.project_level {
        if args.track.is_some() {
            bail!("give --track or --project, not both");
        }
        format!("{}/decisions?scope=project", project_path(&agent, &project))
    } else if let Some(given) = args.part.as_deref() {
        if args.track.is_some() {
            bail!("give --track or --part, not both");
        }
        let part = resolve_part(&agent, &project, given.trim())?;
        format!("{}/parts/{part}/decisions", project_path(&agent, &project))
    } else if let Some(given) = args.feature.as_deref() {
        if args.track.is_some() {
            bail!("give --track or --feature, not both");
        }
        let feature = resolve_feature(&agent, &project, given, None)?;
        format!(
            "{}/parts/{}/features/{}/decisions",
            project_path(&agent, &project),
            feature.part_id,
            feature.id()
        )
    } else {
        let Some(given) = current_track(args) else {
            bail!(
                "omni decision list needs a track, a feature, a part or the project: --track <name|id>, --feature <name|id>, --part <name|id>, --project or OMNI_TRACK"
            );
        };
        let track = resolve_track(&agent, &project, &given)?;
        format!(
            "{}/tracks/{track}/decisions",
            project_path(&agent, &project)
        )
    };
    let response = agent.request("GET", &path, None)?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let body = parsed(&text, "the decisions")?;
    let decisions = body
        .get("decisions")
        .and_then(Value::as_array)
        .unwrap_or_default();
    if decisions.is_empty() {
        say("no decisions");
    }
    for decision in decisions {
        let field = |name: &str| decision.str_at(name).unwrap_or("?");
        let about = match decision.str_at("about") {
            Some("feature") => decision
                .str_at("feature_name")
                .or(decision.str_at("feature_id"))
                .unwrap_or("?")
                .to_string(),
            Some("part") => format!(
                "part {}",
                decision
                    .str_at("part_name")
                    .or(decision.str_at("part_id"))
                    .unwrap_or("?")
            ),
            Some("project") => "project".to_string(),
            _ => "-".to_string(),
        };
        let mut line = format!(
            "{}  {}  {about}  {} -> {} (why: {})",
            field("id"),
            field("created_at"),
            field("question"),
            field("decision"),
            field("why")
        );
        if let Some(id) = decision.str_at("replaces_decision_id") {
            line.push_str(&format!("  replaces {id}"));
        }
        if let Some(id) = decision.str_at("replaced_by_decision_id") {
            line.push_str(&format!("  replaced by {id}"));
        }
        say(&line);
    }
    Ok(())
}
