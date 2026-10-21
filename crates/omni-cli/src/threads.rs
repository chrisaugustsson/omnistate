//! `omni thread create` and `omni thread phase`: start a thread, a question
//! that agents work out in a change set, and start the thread's next phase.
//! The project is found as for `omni changeset`.

use crate::args::Args;
use crate::changesets::{changeset_id, is_uuid, project_id};
use crate::commands::{agent, say};
use crate::error::{CliError, Result, bail};
use crate::json::{Value, quote};

const CREATE_USAGE: &str =
    "omni thread create --changeset <id> --question <text> --criteria-file <path> [--sealed]";
const PHASE_USAGE: &str = "omni thread phase <thread> --open|--sealed";
const DIGEST_USAGE: &str = "omni thread digest <thread>";

/// `open` or `sealed`: what a phase of the answer's thread is.
fn kind(phase: &Value) -> &'static str {
    if matches!(phase.get("sealed"), Some(Value::Bool(true))) {
        "sealed"
    } else {
        "open"
    }
}

/// The criteria in a file: each line that is not blank is one criterion.
fn criteria_in(path: &str) -> Result<Vec<String>> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| CliError::new(format!("cannot read the criteria file {path}: {e}")))?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

/// `omni thread create`
pub fn create(args: &Args) -> Result<()> {
    let Some(changeset) = args.changeset.as_deref() else {
        bail!("this command needs --changeset <id>: {CREATE_USAGE}");
    };
    let changeset = changeset_id(changeset)?;
    let question = args.question.as_deref().map(str::trim).unwrap_or_default();
    if question.is_empty() {
        bail!("this command needs --question <text>: {CREATE_USAGE}");
    }
    let Some(file) = args.criteria_file.as_deref() else {
        bail!("this command needs --criteria-file <path>: {CREATE_USAGE}");
    };
    let criteria = criteria_in(file)?;

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let criteria: Vec<String> = criteria.iter().map(|c| quote(c)).collect();
    let body = format!(
        "{{\"question\":{},\"criteria\":[{}],\"sealed\":{}}}",
        quote(question),
        criteria.join(","),
        args.sealed
    );
    let not_found = format!(
        "not_found: no change set {changeset} in this project; `omni changeset list` shows them"
    );
    let response = agent.request_or_not_found(
        "POST",
        &format!(
            "/api/v1/w/{}/projects/{project}/changesets/{changeset}/threads",
            agent.workspace_id
        ),
        Some(body.as_bytes()),
        &not_found,
    )?;
    answer(args, &response.text(), |thread, phase| {
        format!(
            "started thread {}  phase 1 is {}",
            thread.str_at("id").unwrap_or("?"),
            kind(phase)
        )
    })
}

/// `omni thread phase <thread> --open|--sealed`
pub fn phase(args: &Args, thread: Option<&str>) -> Result<()> {
    let Some(thread) = thread else {
        bail!("this command needs the thread's ID: {PHASE_USAGE}");
    };
    if !is_uuid(thread) {
        bail!("{thread:?} is not a thread ID: {PHASE_USAGE}");
    }
    let thread = thread.to_ascii_lowercase();
    if args.open == args.sealed {
        bail!("this command needs exactly one of --open and --sealed: {PHASE_USAGE}");
    }

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let body = format!("{{\"sealed\":{}}}", args.sealed);
    let not_found = format!("not_found: no thread {thread} in this project");
    let response = agent.request_or_not_found(
        "POST",
        &format!(
            "/api/v1/w/{}/projects/{project}/threads/{thread}/phases",
            agent.workspace_id
        ),
        Some(body.as_bytes()),
        &not_found,
    )?;
    answer(args, &response.text(), |thread, phase| {
        format!(
            "thread {}  phase {} is {}",
            thread.str_at("id").unwrap_or("?"),
            phase
                .get("number")
                .and_then(|n| match n {
                    Value::Number(n) => Some(n.as_str()),
                    _ => None,
                })
                .unwrap_or("?"),
            kind(phase)
        )
    })
}

/// Prints the API's answer, as it is with `--json`, else one line made from
/// the thread and its current (last) phase.
fn answer(args: &Args, text: &str, line: impl Fn(&Value, &Value) -> String) -> Result<()> {
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    let unreadable = || {
        CliError::new(
            "the server's answer about the thread is not readable; is it an OmniState API?",
        )
    };
    let body = Value::parse(text).map_err(|_| unreadable())?;
    let thread = body.get("thread").unwrap_or(&body);
    let phase = thread
        .get("phases")
        .and_then(Value::as_array)
        .and_then(<[Value]>::last)
        .ok_or_else(unreadable)?;
    say(&line(thread, phase));
    Ok(())
}

/// `omni thread digest <thread>`: one line for each post the caller may read,
/// oldest first: its type, its author's role, its summary and its ID. It reads
/// nothing but the digest, the cheap view of a thread.
pub fn digest(args: &Args, thread: Option<&str>) -> Result<()> {
    let Some(thread) = thread else {
        bail!("this command needs the thread's ID: {DIGEST_USAGE}");
    };
    if !is_uuid(thread) {
        bail!("{thread:?} is not a thread ID: {DIGEST_USAGE}");
    }
    let thread = thread.to_ascii_lowercase();

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let not_found = format!("not_found: no thread {thread} in this project");
    let response = agent.request_or_not_found(
        "GET",
        &format!(
            "/api/v1/w/{}/projects/{project}/threads/{thread}/digest",
            agent.workspace_id
        ),
        None,
        &not_found,
    )?;
    let text = response.text();
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    let unreadable = || {
        CliError::new(
            "the server's answer about the thread is not readable; is it an OmniState API?",
        )
    };
    let body = Value::parse(&text).map_err(|_| unreadable())?;
    let posts = body
        .get("posts")
        .and_then(Value::as_array)
        .ok_or_else(unreadable)?;
    for post in posts {
        say(&format!(
            "{}  {}  {}  {}",
            post.str_at("type").unwrap_or("?"),
            crate::posts::role(post),
            post.str_at("summary").unwrap_or_default(),
            post.str_at("id").unwrap_or("?")
        ));
    }
    Ok(())
}
