//! `omni comment list` and `omni comment reply`: the comments that the human
//! pins to nodes of the live docs. An agent lists them and answers with a
//! reply; the human deletes a comment when satisfied, in the web app. A comment
//! has no status. The project is found as for `omni changeset`.

use crate::args::Args;
use crate::changesets::{is_uuid, project_id, resolve_feature};
use crate::commands::{agent, read_body, say};
use crate::error::{CliError, Result, bail};
use crate::json::{Value, quote};
use std::io::IsTerminal;

const LIST_USAGE: &str = "omni comment list [--doc <id>] [--feature <name|id>]";
const REPLY_USAGE: &str = "omni comment reply <comment> <text>";

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

/// Who wrote a comment or a reply, for a line of the list.
fn who(author: Option<&Value>) -> String {
    let Some(author) = author else {
        return "?".to_string();
    };
    match author.str_at("kind") {
        Some("agent") => format!(
            "agent {}/{}",
            author.str_at("run").unwrap_or("?"),
            author.str_at("role").unwrap_or("?")
        ),
        Some(kind) => kind.to_string(),
        None => "?".to_string(),
    }
}

/// The lines of `text`, each indented by `indent`.
fn indented(text: &str, indent: &str) -> String {
    text.lines()
        .map(|line| format!("{indent}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `omni comment list [--doc <id>] [--feature <name or id>]`
pub fn list(args: &Args) -> Result<()> {
    let doc = args
        .doc
        .as_deref()
        .map(|doc| id_of(doc, "a doc ID for --doc", LIST_USAGE))
        .transpose()?;

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let feature = args
        .feature
        .as_deref()
        .map(|given| resolve_feature(&agent, &project, given))
        .transpose()?;
    let mut query = Vec::new();
    if let Some(doc) = doc {
        query.push(format!("doc={doc}"));
    }
    if let Some(feature) = feature {
        query.push(format!("feature={feature}"));
    }
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
        let waiting = match comment.get("waiting_on_you") {
            Some(Value::Bool(true)) => "  waiting on you",
            _ => "",
        };
        say(&format!(
            "{}  doc {}  node {}  {}{waiting}",
            comment.str_at("id").unwrap_or("?"),
            comment.str_at("doc_id").unwrap_or("?"),
            comment.str_at("node").unwrap_or("?"),
            who(comment.get("author")),
        ));
        say(&indented(comment.str_at("text").unwrap_or_default(), "  "));
        for reply in comment
            .get("replies")
            .and_then(Value::as_array)
            .unwrap_or_default()
        {
            say(&format!("  reply, {}:", who(reply.get("author"))));
            say(&indented(reply.str_at("text").unwrap_or_default(), "    "));
        }
    }
    Ok(())
}

/// The text of the reply: the argument, else `--body-file`, else all of
/// standard input.
fn reply_text(args: &Args, given: Option<&str>) -> Result<String> {
    if let Some(text) = given {
        return Ok(text.to_string());
    }
    let bytes = match args.body_file.as_deref() {
        Some(path) => read_body(path)?,
        None => {
            if std::io::stdin().is_terminal() {
                bail!(
                    "this command needs the reply's text: pass it after the comment's ID, with --body-file <path>, or pipe it on standard input: {REPLY_USAGE}"
                );
            }
            read_body("-")?
        }
    };
    String::from_utf8(bytes).map_err(|_| CliError::new("the text of the reply is not UTF-8 text"))
}

/// `omni comment reply <comment> <text>`
pub fn reply(args: &Args, comment: Option<&str>, text: Option<&str>) -> Result<()> {
    let Some(comment) = comment else {
        bail!("this command needs the comment's ID: {REPLY_USAGE}");
    };
    let comment = id_of(comment, "a comment ID", REPLY_USAGE)?;
    let text = reply_text(args, text)?;
    let text = text.trim();
    if text.is_empty() {
        bail!("the reply's text is blank; write it after the comment's ID: {REPLY_USAGE}");
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

/// `omni comment addressed|resolve|reopen`: comments have no status.
pub fn no_status(command: &str) -> Result<()> {
    bail!(
        "`omni comment {command}` is gone: comments have no status; reply with `omni comment reply`, and the human deletes a comment when satisfied, in the web app"
    )
}
