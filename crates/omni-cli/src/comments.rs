//! `omni comment list`, `omni comment reply` and `omni comment addressed`: the
//! comments that the human pins to nodes of saved versions. An agent lists them,
//! answers with a reply and marks a comment addressed once a later version fixes
//! it; resolving and reopening are the human's, in the web app. The project is
//! found as for `omni changeset`.

use crate::args::Args;
use crate::changesets::{is_uuid, project_id};
use crate::commands::{agent, read_body, say};
use crate::error::{CliError, Result, bail};
use crate::json::{Value, quote};
use std::io::IsTerminal;

const LIST_USAGE: &str =
    "omni comment list [--doc <id> | --changeset <id>] [--status open|addressed|resolved]";
const REPLY_USAGE: &str = "omni comment reply <comment> [--body-file <path>]";
const ADDRESSED_USAGE: &str = "omni comment addressed <comment> --version <version>";
const STATUSES: [&str; 3] = ["open", "addressed", "resolved"];

fn unreadable(what: &str) -> CliError {
    CliError::new(format!(
        "the server's answer about {what} is not readable; is it an OmniState API?"
    ))
}

/// The ID that an option or an argument names, in lowercase.
fn id_of(text: &str, what: &str, usage: &str) -> Result<String> {
    if is_uuid(text) {
        Ok(text.to_ascii_lowercase())
    } else {
        bail!("{text:?} is not {what} (a UUID): {usage}")
    }
}

/// `omni comment list [--doc <id> | --changeset <id>] [--status <status>]`
pub fn list(args: &Args) -> Result<()> {
    let mut query = Vec::new();
    match (args.doc.as_deref(), args.changeset.as_deref()) {
        (Some(_), Some(_)) => bail!(
            "--doc and --changeset cannot be used together; list one doc's comments or one change set's: {LIST_USAGE}"
        ),
        (Some(doc), None) => query.push(format!(
            "doc={}",
            id_of(doc, "a doc ID for --doc", LIST_USAGE)?
        )),
        (None, Some(changeset)) => query.push(format!(
            "changeset={}",
            id_of(changeset, "a change set ID for --changeset", LIST_USAGE)?
        )),
        (None, None) => {}
    }
    if let Some(status) = args.status.as_deref() {
        if !STATUSES.contains(&status) {
            bail!(
                "{status:?} is not a status for --status; use one of {}",
                STATUSES.join(", ")
            );
        }
        query.push(format!("status={status}"));
    }

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let mut path = format!(
        "/api/v1/w/{}/projects/{project}/comments",
        agent.workspace_id
    );
    if !query.is_empty() {
        path.push('?');
        path.push_str(&query.join("&"));
    }
    let response = agent.request("GET", &path, None)?;
    let text = response.text();
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    let answer = Value::parse(&text).map_err(|_| unreadable("the comments"))?;
    for comment in answer
        .get("comments")
        .and_then(Value::as_array)
        .unwrap_or_default()
    {
        let marker = match comment.str_at("node_status") {
            Some("changed") => "  [changed]",
            Some("removed") => "  [removed]",
            _ => "",
        };
        say(&format!(
            "{}  {}  {}{marker}  {}",
            comment.str_at("id").unwrap_or("?"),
            comment.str_at("status").unwrap_or("?"),
            comment.str_at("node").unwrap_or("?"),
            comment
                .str_at("text")
                .and_then(|text| text.lines().next())
                .unwrap_or_default()
        ));
    }
    Ok(())
}

/// The text of the reply: `--body-file`, or else all of standard input.
fn reply_text(args: &Args) -> Result<String> {
    let bytes = match args.body_file.as_deref() {
        Some(path) => read_body(path)?,
        None => {
            if std::io::stdin().is_terminal() {
                bail!(
                    "this command needs the reply's text: pass --body-file <path> or pipe it on standard input: {REPLY_USAGE}"
                );
            }
            read_body("-")?
        }
    };
    String::from_utf8(bytes).map_err(|_| CliError::new("the text of the reply is not UTF-8 text"))
}

/// `omni comment reply <comment>`
pub fn reply(args: &Args, comment: Option<&str>) -> Result<()> {
    let Some(comment) = comment else {
        bail!("this command needs the comment's ID: {REPLY_USAGE}");
    };
    let comment = id_of(comment, "a comment ID", REPLY_USAGE)?;
    let text = reply_text(args)?;
    let text = text.trim();
    if text.is_empty() {
        bail!("the reply's text is blank; write the reply on standard input or in --body-file");
    }

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let not_found = format!(
        "not_found: no comment {comment} in this project; `omni comment list` shows the comments"
    );
    let response = agent.request_or_not_found(
        "POST",
        &format!(
            "/api/v1/w/{}/projects/{project}/comments/{comment}/replies",
            agent.workspace_id
        ),
        Some(format!("{{\"text\":{}}}", quote(text)).as_bytes()),
        &not_found,
    )?;
    let answer = response.text();
    if args.json {
        say(answer.trim_end());
        return Ok(());
    }
    say(&format!("replied to comment {comment}"));
    Ok(())
}

/// `omni comment addressed <comment> --version <version>`
pub fn addressed(args: &Args, comment: Option<&str>) -> Result<()> {
    let Some(comment) = comment else {
        bail!("this command needs the comment's ID: {ADDRESSED_USAGE}");
    };
    let comment = id_of(comment, "a comment ID", ADDRESSED_USAGE)?;
    let Some(version) = args.doc_version.as_deref() else {
        bail!(
            "this command needs --version <version>, the version that fixes the comment: {ADDRESSED_USAGE}"
        );
    };
    let version = id_of(version, "a version ID for --version", ADDRESSED_USAGE)?;

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let not_found = format!(
        "not_found: no comment {comment} in this project; `omni comment list` shows the comments"
    );
    let response = agent.request_or_not_found(
        "POST",
        &format!(
            "/api/v1/w/{}/projects/{project}/comments/{comment}/addressed",
            agent.workspace_id
        ),
        Some(format!("{{\"version_id\":{}}}", quote(&version)).as_bytes()),
        &not_found,
    )?;
    let answer = response.text();
    if args.json {
        say(answer.trim_end());
        return Ok(());
    }
    say(&format!("comment {comment} addressed in version {version}"));
    Ok(())
}

/// `omni comment resolve` and `omni comment reopen`: the human's. `doing` is
/// what the command would do, as in "resolving".
pub fn human_only(doing: &str) -> Result<()> {
    bail!("{doing} a comment is the human's decision and is made in the web app; an agent cannot")
}
