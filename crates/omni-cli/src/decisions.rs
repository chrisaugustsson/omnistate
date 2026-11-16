//! `omni decision`: record a decision on a track and list the decisions of a
//! track or a feature. A decision says what was decided and why, may be about
//! one feature, and may replace an earlier decision; it is never edited or
//! deleted.

use crate::args::Args;
use crate::commands::{Agent, agent, say};
use crate::error::{CliError, Result, bail};
use crate::features::{project_features, resolve_feature};
use crate::json::{Value, quote};
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

/// `omni decision add --question <q> --decision <d> --why <w> [--feature <f>] [--replaces <id>]`
pub fn add(args: &Args) -> Result<()> {
    const USAGE: &str = "omni decision add --question <q> --decision <d> --why <w> [--feature <name|id>] [--replaces <id>]";
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
            resolve_feature(&agent, &project, given, args.part.as_deref())?
                .id()
                .to_string()
        };
        body.push_str(&format!(",\"feature_id\":{}", quote(&feature)));
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

/// `omni decision list [--track <name|id> | --feature <name|id>]`
pub fn list(args: &Args) -> Result<()> {
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let path = if let Some(given) = args.feature.as_deref() {
        if args.track.is_some() {
            bail!("give --track or --feature, not both");
        }
        let feature = resolve_feature(&agent, &project, given, args.part.as_deref())?;
        format!(
            "{}/parts/{}/features/{}/decisions",
            project_path(&agent, &project),
            feature.part_id,
            feature.id()
        )
    } else {
        let Some(given) = current_track(args) else {
            bail!(
                "omni decision list needs a track or a feature: --track <name|id>, --feature <name|id> or OMNI_TRACK"
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
    // The names of the features are only looked up when a decision is about one.
    let features = if decisions
        .iter()
        .any(|decision| decision.str_at("feature_id").is_some())
    {
        project_features(&agent, &project, None).unwrap_or_default()
    } else {
        Vec::new()
    };
    for decision in decisions {
        let field = |name: &str| decision.str_at(name).unwrap_or("?");
        let feature = match decision.str_at("feature_id") {
            None => "-",
            Some(id) => features
                .iter()
                .find(|found| found.id() == id)
                .and_then(|found| found.feature.str_at("name"))
                .unwrap_or(id),
        };
        let mut line = format!(
            "{}  {}  {feature}  {} -> {} (why: {})",
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
