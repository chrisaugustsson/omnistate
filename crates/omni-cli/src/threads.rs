//! `omni thread create`, `list`, `read`, `conclude` and `digest`: threads at the
//! app level of a project or in one of its features. A
//! thread is a question that agents work out together: they post to it with
//! `omni post`, and one of them writes the conclusion. A sealed thread hides
//! the posts of other agents from an agent until it has posted itself. The
//! project is found as for `omni project`.

use crate::args::Args;
use crate::commands::{agent, say};
use crate::error::{CliError, Result, bail};
use crate::json::{Value, quote};
use crate::projects::{creation_feature, is_uuid, project_id};

const CREATE_USAGE: &str = "omni thread create <question> [--feature <name or id>] [--sealed]";
const READ_USAGE: &str = "omni thread read <thread>";
const CONCLUDE_USAGE: &str = "omni thread conclude <thread> <text>";
const DIGEST_USAGE: &str = "omni thread digest <thread>";

fn unreadable() -> CliError {
    CliError::new("the server's answer about the thread is not readable; is it an OmniState API?")
}

/// The ID of the thread that an argument names, in lowercase.
fn thread_id(thread: &str, usage: &str) -> Result<String> {
    if is_uuid(thread) {
        Ok(thread.to_ascii_lowercase())
    } else {
        bail!("{thread:?} is not a thread ID: {usage}")
    }
}

/// `omni thread create <question> [--feature <name or id>] [--sealed]`
pub fn create(args: &Args, question: Option<&str>) -> Result<()> {
    let question = question.map(str::trim).unwrap_or_default();
    if question.is_empty() {
        bail!("this command needs the thread's question: {CREATE_USAGE}");
    }

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let feature = creation_feature(args, &agent, &project)?;
    let body = format!(
        "{{\"question\":{},\"sealed\":{}}}",
        quote(question),
        args.sealed
    );
    let base = format!("/api/v1/w/{}/projects/{project}", agent.workspace_id);
    let (path, not_found) = match feature {
        Some(feature) => (
            format!("{base}/features/{feature}/threads"),
            format!(
                "not_found: no feature {feature} in this project; `omni feature list` shows them"
            ),
        ),
        None => (
            format!("{base}/threads"),
            format!("not_found: no project {project} in this workspace"),
        ),
    };
    let response = agent.request_or_not_found("POST", &path, Some(body.as_bytes()), &not_found)?;
    let text = response.text();
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    let answer = Value::parse(&text).map_err(|_| unreadable())?;
    match answer.get("thread").unwrap_or(&answer).str_at("id") {
        Some(id) => say(id),
        None => bail!("the server's answer has no thread ID; try again"),
    }
    Ok(())
}

/// What a thread is, for a line of the list: its scope, and whether it is
/// sealed or concluded.
fn state(thread: &Value) -> String {
    let mut state = thread.str_at("scope").unwrap_or("?").to_string();
    if matches!(thread.get("sealed"), Some(Value::Bool(true))) {
        state.push_str(", sealed");
    }
    if thread.str_at("conclusion").is_some() {
        state.push_str(", concluded");
    }
    state
}

/// `omni thread list`: one line for each thread of the project, of every
/// scope, oldest first: its ID, what it is and its question.
pub fn list(args: &Args) -> Result<()> {
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let response = agent.request(
        "GET",
        &format!(
            "/api/v1/w/{}/projects/{project}/threads",
            agent.workspace_id
        ),
        None,
    )?;
    let text = response.text();
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    let body = Value::parse(&text).map_err(|_| unreadable())?;
    let threads = body
        .get("threads")
        .and_then(Value::as_array)
        .ok_or_else(unreadable)?;
    for thread in threads {
        say(&format!(
            "{}  {}  {}",
            thread.str_at("id").unwrap_or("?"),
            state(thread),
            thread.str_at("question").unwrap_or_default()
        ));
    }
    Ok(())
}

/// `omni thread read <thread>`: the question, every post the agent may read
/// (a sealed thread hides the others' posts until the agent has posted) and
/// the conclusion when there is one.
pub fn read(args: &Args, thread: Option<&str>) -> Result<()> {
    let Some(thread) = thread else {
        bail!("this command needs the thread's ID: {READ_USAGE}");
    };
    let thread = thread_id(thread, READ_USAGE)?;

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let not_found = format!("not_found: no thread {thread} in this project");
    let response = agent.request_or_not_found(
        "GET",
        &format!(
            "/api/v1/w/{}/projects/{project}/threads/{thread}",
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
    let body = Value::parse(&text).map_err(|_| unreadable())?;
    let shown = body.get("thread").unwrap_or(&body);
    say(&format!(
        "{}  {}",
        shown.str_at("id").unwrap_or("?"),
        state(shown)
    ));
    say(&format!(
        "question: {}",
        shown.str_at("question").unwrap_or_default()
    ));
    for post in shown
        .get("posts")
        .and_then(Value::as_array)
        .unwrap_or_default()
    {
        say("");
        say(&format!(
            "{}  {}  {}",
            crate::posts::role(post),
            post.str_at("summary").unwrap_or_default(),
            post.str_at("id").unwrap_or("?")
        ));
        for line in post.str_at("body").unwrap_or_default().lines() {
            say(&format!("  {line}"));
        }
    }
    if let Some(conclusion) = shown.str_at("conclusion") {
        say("");
        say(&format!("conclusion: {conclusion}"));
    }
    Ok(())
}

/// `omni thread conclude <thread> <text>`: writes the thread's conclusion. The
/// thread keeps taking posts.
pub fn conclude(args: &Args, thread: Option<&str>, text: Option<&str>) -> Result<()> {
    let (Some(thread), Some(text)) = (thread, text) else {
        bail!("this command needs the thread's ID and the conclusion: {CONCLUDE_USAGE}");
    };
    let thread = thread_id(thread, CONCLUDE_USAGE)?;
    let text = text.trim();
    if text.is_empty() {
        bail!("the conclusion is blank: {CONCLUDE_USAGE}");
    }

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let body = format!("{{\"conclusion\":{}}}", quote(text));
    let not_found = format!("not_found: no thread {thread} in this project");
    let response = agent.request_or_not_found(
        "PUT",
        &format!(
            "/api/v1/w/{}/projects/{project}/threads/{thread}/conclusion",
            agent.workspace_id
        ),
        Some(body.as_bytes()),
        &not_found,
    )?;
    let answer = response.text();
    if args.json {
        say(answer.trim_end());
        return Ok(());
    }
    say(&format!("concluded thread {thread}"));
    Ok(())
}

/// `omni thread digest <thread>`: one line for each post the caller may read,
/// oldest first: its author's role, its summary and its ID. It reads nothing but the digest, the
/// cheap view of a thread.
pub fn digest(args: &Args, thread: Option<&str>) -> Result<()> {
    let Some(thread) = thread else {
        bail!("this command needs the thread's ID: {DIGEST_USAGE}");
    };
    let thread = thread_id(thread, DIGEST_USAGE)?;

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
    let body = Value::parse(&text).map_err(|_| unreadable())?;
    let posts = body
        .get("posts")
        .and_then(Value::as_array)
        .ok_or_else(unreadable)?;
    for post in posts {
        say(&format!(
            "{}  {}  {}",
            crate::posts::role(post),
            post.str_at("summary").unwrap_or_default(),
            post.str_at("id").unwrap_or("?")
        ));
    }
    Ok(())
}
