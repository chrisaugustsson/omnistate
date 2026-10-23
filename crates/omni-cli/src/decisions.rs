//! `omni decision propose`, `omni decision show` and `omni changeset
//! link-decision`: the decision record a thread ends in. An agent proposes the
//! decision from a JSON file (the schema is in docs/formats/decision.md), reads
//! it back and links a change set to it; approving and rejecting it are the
//! human's, in the web app. The project is found as for `omni changeset`.

use crate::args::Args;
use crate::changesets::{changeset_id, is_uuid, project_id};
use crate::commands::{agent, checked, say};
use crate::error::{CliError, Result, bail};
use crate::json::Value;

const PROPOSE_USAGE: &str = "omni decision propose <thread> --file <decision.json>";
const SHOW_USAGE: &str = "omni decision show <thread|decision>";
const LINK_USAGE: &str = "omni changeset link-decision <changeset> <decision>";

fn unreadable(what: &str) -> CliError {
    CliError::new(format!(
        "the server's answer about {what} is not readable; is it an OmniState API?"
    ))
}

/// `omni decision propose <thread> --file <decision.json>`
pub fn propose(args: &Args, thread: Option<&str>) -> Result<()> {
    let Some(thread) = thread else {
        bail!("this command needs the thread's ID: {PROPOSE_USAGE}");
    };
    if !is_uuid(thread) {
        bail!("{thread:?} is not a thread ID: {PROPOSE_USAGE}");
    }
    let thread = thread.to_ascii_lowercase();
    let Some(file) = args.file.as_deref() else {
        bail!("this command needs --file <decision.json>: {PROPOSE_USAGE}");
    };
    let bytes = std::fs::read(file)
        .map_err(|e| CliError::new(format!("cannot read the decision file {file}: {e}")))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| CliError::new(format!("the decision file {file} is not UTF-8 text")))?;
    match Value::parse(&text) {
        Ok(Value::Object(_)) => {}
        Ok(_) => bail!(
            "the decision file {file} is not a JSON object; docs/formats/decision.md has the schema"
        ),
        Err(error) => bail!(
            "the decision file {file} is not JSON ({}); docs/formats/decision.md has the schema",
            error.lines().next().unwrap_or_default()
        ),
    }

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let not_found = format!("not_found: no thread {thread} in this project");
    let response = agent.request_or_not_found(
        "POST",
        &format!(
            "/api/v1/w/{}/projects/{project}/threads/{thread}/decisions",
            agent.workspace_id
        ),
        Some(text.as_bytes()),
        &not_found,
    )?;
    let answer = response.text();
    if args.json {
        say(answer.trim_end());
        return Ok(());
    }
    let answer = Value::parse(&answer).map_err(|_| unreadable("the decision"))?;
    let decision = answer.get("decision").unwrap_or(&answer);
    match decision.str_at("id") {
        Some(id) => say(&format!(
            "{id}  {}",
            decision.str_at("status").unwrap_or("?")
        )),
        None => bail!("the server's answer has no decision ID; try again"),
    }
    Ok(())
}

/// `omni decision show <thread|decision>`: the decision with this ID, or else
/// the current decision of the thread with this ID.
pub fn show(args: &Args, id: Option<&str>) -> Result<()> {
    let Some(id) = id else {
        bail!("this command needs a thread's or a decision's ID: {SHOW_USAGE}");
    };
    if !is_uuid(id) {
        bail!("{id:?} is neither a thread ID nor a decision ID: {SHOW_USAGE}");
    }
    let id = id.to_ascii_lowercase();

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let base = format!("/api/v1/w/{}/projects/{project}", agent.workspace_id);
    let response = agent.request_any("GET", &format!("{base}/decisions/{id}"), None)?;
    let (text, decision) = if response.status == 404 {
        let response = agent.request_any("GET", &format!("{base}/threads/{id}"), None)?;
        if response.status == 404 {
            bail!("not_found: no decision or thread {id} in this project");
        }
        let text = checked(response)?.text();
        let thread = Value::parse(&text).map_err(|_| unreadable("the thread"))?;
        let decision = thread
            .get("thread")
            .unwrap_or(&thread)
            .get("decision")
            .cloned()
            .unwrap_or(Value::Null);
        (format!("{{\"decision\":{decision}}}"), decision)
    } else {
        let text = checked(response)?.text();
        let answer = Value::parse(&text).map_err(|_| unreadable("the decision"))?;
        let decision = answer.get("decision").cloned().unwrap_or(Value::Null);
        (text, decision)
    };
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    if matches!(decision, Value::Null) {
        say("no decision");
        return Ok(());
    }
    print(&decision);
    Ok(())
}

/// A list of texts under a heading, one `- text` line each; nothing when empty.
fn list(heading: &str, items: &[Value], line: impl Fn(&Value) -> String) {
    if items.is_empty() {
        return;
    }
    say(&format!("{heading}:"));
    for item in items {
        say(&format!("  - {}", line(item)));
    }
}

fn items<'a>(decision: &'a Value, field: &str) -> &'a [Value] {
    decision
        .get(field)
        .and_then(Value::as_array)
        .unwrap_or_default()
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or("?").to_string()
}

/// The decision as text: the header line, then every part of it.
fn print(decision: &Value) {
    let field = |name: &str| decision.str_at(name).unwrap_or("?");
    say(&format!("decision {}  {}", field("id"), field("status")));
    say(&format!("thread: {}", field("thread_id")));
    say(&format!("direction: {}", field("direction")));
    say(&format!("chosen: {}", field("chosen")));
    let criteria: Vec<String> = items(decision, "criteria").iter().map(text).collect();
    say(&format!("criteria: {}", criteria.join(", ")));
    say("options:");
    for option in items(decision, "options") {
        let scores = match option.get("scores") {
            Some(Value::Object(scores)) => scores
                .iter()
                .map(|(criterion, score)| format!("{criterion} {score}"))
                .collect::<Vec<_>>()
                .join(", "),
            _ => String::new(),
        };
        let name = option.str_at("name").unwrap_or("?");
        let chosen = if option.str_at("name") == decision.str_at("chosen") {
            "  (chosen)"
        } else {
            ""
        };
        say(&format!("  - {name}{chosen}  [{scores}]"));
        if let Some(why) = option.str_at("lost_because") {
            say(&format!("      lost because: {why}"));
        }
    }
    list("assumptions", items(decision, "assumptions"), text);
    list("open questions", items(decision, "open_questions"), text);
    list("dissent", items(decision, "dissent"), |dissent| {
        format!(
            "{}: {}",
            dissent.str_at("by").unwrap_or("?"),
            dissent.str_at("text").unwrap_or("?")
        )
    });
    list("posts", items(decision, "posts"), text);
    if let Some(reason) = decision.str_at("rejection_reason") {
        say(&format!("rejection reason: {reason}"));
    }
}

/// `omni changeset link-decision <changeset> <decision>`
pub fn link(args: &Args, changeset: Option<&str>, decision: Option<&str>) -> Result<()> {
    let (Some(changeset), Some(decision)) = (changeset, decision) else {
        bail!("this command needs a change set's ID and a decision's ID: {LINK_USAGE}");
    };
    let changeset = changeset_id(changeset)?;
    if !is_uuid(decision) {
        bail!("{decision:?} is not a decision ID: {LINK_USAGE}");
    }
    let decision = decision.to_ascii_lowercase();

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let not_found = format!(
        "not_found: no change set {changeset} or decision {decision} in this project; `omni changeset list` shows the change sets"
    );
    let response = agent.request_or_not_found(
        "POST",
        &format!(
            "/api/v1/w/{}/projects/{project}/changesets/{changeset}/decisions/{decision}",
            agent.workspace_id
        ),
        None,
        &not_found,
    )?;
    let answer = response.text();
    if args.json {
        say(answer.trim_end());
        return Ok(());
    }
    say(&format!(
        "change set {changeset} carries out decision {decision}"
    ));
    Ok(())
}
