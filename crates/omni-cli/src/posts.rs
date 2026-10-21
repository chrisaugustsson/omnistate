//! `omni post <thread>`: posts to a thread, a typed message with a one-line
//! summary and a Markdown body. The body is read from `--body-file` or else
//! standard input, each `--attach` file is uploaded first, and everything that
//! can be checked here is checked before anything is sent. The project is found
//! as for `omni changeset`.

use crate::args::Args;
use crate::attach;
use crate::changesets::{is_uuid, project_id};
use crate::commands::{agent, read_body, say};
use crate::error::{CliError, Result, bail};
use crate::json::{Value, quote};
use std::io::IsTerminal;
use std::path::Path;

const USAGE: &str = "omni post <thread> --type <type> --summary <text> [--body-file <path>] \
     [--attach <file>]... [--link <target>]... [--confidence <0-1>] [--replaces <post>]";
/// The types an agent may post; `directive` is the human's.
const TYPES: [&str; 7] = [
    "note",
    "finding",
    "position",
    "objection",
    "support",
    "question",
    "verdict",
];
const MAX_SUMMARY_CHARS: usize = 200;

/// The `--link` target as the JSON object the API takes.
fn link(target: &str) -> Result<String> {
    let bad = || {
        CliError::new(format!(
            "{target:?} is not a link for --link; use post:<id>, attachment:<id>, node:<doc>/<address> or an http(s) address"
        ))
    };
    let lower = target.to_ascii_lowercase();
    if let Some(id) = target.strip_prefix("post:") {
        is_uuid(id)
            .then(|| {
                format!(
                    "{{\"kind\":\"post\",\"post_id\":{}}}",
                    quote(&id.to_ascii_lowercase())
                )
            })
            .ok_or_else(bad)
    } else if let Some(id) = target.strip_prefix("attachment:") {
        is_uuid(id)
            .then(|| {
                format!(
                    "{{\"kind\":\"attachment\",\"attachment_id\":{}}}",
                    quote(&id.to_ascii_lowercase())
                )
            })
            .ok_or_else(bad)
    } else if let Some(node) = target.strip_prefix("node:") {
        let (doc, address) = node.split_once('/').ok_or_else(bad)?;
        (is_uuid(doc) && !address.is_empty())
            .then(|| {
                format!(
                    "{{\"kind\":\"node\",\"doc_id\":{},\"address\":{}}}",
                    quote(&doc.to_ascii_lowercase()),
                    quote(address)
                )
            })
            .ok_or_else(bad)
    } else if (lower.starts_with("http://") || lower.starts_with("https://"))
        && !target.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        Ok(format!("{{\"kind\":\"url\",\"url\":{}}}", quote(target)))
    } else {
        Err(bad())
    }
}

/// The text of the post: `--body-file`, or else all of standard input.
fn body(args: &Args) -> Result<String> {
    let bytes = match args.body_file.as_deref() {
        Some(path) => read_body(path)?,
        None => {
            if std::io::stdin().is_terminal() {
                bail!(
                    "this command needs the post's body: pass --body-file <path> or pipe it on standard input: {USAGE}"
                );
            }
            read_body("-")?
        }
    };
    String::from_utf8(bytes).map_err(|_| CliError::new("the body of the post is not UTF-8 text"))
}

/// A file to attach, read before anything is sent: its name and bytes.
fn attachment(file: &str) -> Result<(String, Vec<u8>)> {
    let unreadable = |e: std::io::Error| {
        CliError::new(format!(
            "cannot read the file {file} for --attach: {e}; check the path"
        ))
    };
    let path = Path::new(file);
    let metadata = std::fs::metadata(path).map_err(unreadable)?;
    if metadata.is_dir() {
        bail!("{file} is a directory; --attach takes a file");
    }
    if metadata.len() > attach::MAX_UPLOAD as u64 {
        bail!("{file} is larger than 20 MB, the limit for an attachment; attach a smaller file");
    }
    let bytes = std::fs::read(path).map_err(unreadable)?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| file.to_string());
    Ok((name, bytes))
}

/// `omni post <thread> ...`
pub fn run(args: &Args, thread: Option<&str>) -> Result<()> {
    let Some(thread) = thread else {
        bail!("this command needs the thread's ID: {USAGE}");
    };
    if !is_uuid(thread) {
        bail!("{thread:?} is not a thread ID: {USAGE}");
    }
    let thread = thread.to_ascii_lowercase();
    let Some(kind) = args.post_type.as_deref() else {
        bail!("this command needs --type <type>: {USAGE}");
    };
    if kind == "directive" {
        bail!(
            "--type directive is not for agents: only the human posts directives, in the web app"
        );
    }
    if !TYPES.contains(&kind) {
        bail!(
            "{kind:?} is not a post type for --type; use one of {}",
            TYPES.join(", ")
        );
    }
    let summary = args.summary.as_deref().map(str::trim).unwrap_or_default();
    if summary.is_empty() {
        bail!("this command needs --summary <text>: {USAGE}");
    }
    if summary.chars().count() > MAX_SUMMARY_CHARS || summary.contains(['\n', '\r']) {
        bail!("--summary is one line of at most {MAX_SUMMARY_CHARS} characters; shorten it");
    }
    let confidence = match args.confidence.as_deref() {
        None => None,
        Some(text) => match text.trim().parse::<f64>() {
            Ok(number) if (0.0..=1.0).contains(&number) => Some(number),
            _ => bail!("{text:?} is not a number from 0 to 1 for --confidence"),
        },
    };
    let links = args
        .link
        .iter()
        .map(|target| link(target))
        .collect::<Result<Vec<_>>>()?;
    let replaces = match args.replaces.as_deref() {
        None => None,
        Some(id) if is_uuid(id) => Some(id.to_ascii_lowercase()),
        Some(id) => bail!("{id:?} is not a post ID for --replaces"),
    };
    let text = body(args)?;
    let files = args
        .attach
        .iter()
        .map(|file| attachment(file))
        .collect::<Result<Vec<_>>>()?;

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let mut attachments = Vec::new();
    for (name, bytes) in &files {
        let response = attach::send(&agent, name, bytes)?;
        let answer = Value::parse(&response.text()).ok();
        let id = answer
            .as_ref()
            .and_then(|answer| answer.get("attachment").unwrap_or(answer).str_at("id"))
            .map(str::to_string);
        match id {
            Some(id) => attachments.push(quote(&id)),
            None => bail!("the server's answer about the attachment {name} has no ID; try again"),
        }
    }

    let mut fields = vec![
        format!("\"type\":{}", quote(kind)),
        format!("\"summary\":{}", quote(summary)),
        format!("\"body\":{}", quote(&text)),
    ];
    if !links.is_empty() {
        fields.push(format!("\"links\":[{}]", links.join(",")));
    }
    if !attachments.is_empty() {
        fields.push(format!("\"attachments\":[{}]", attachments.join(",")));
    }
    if let Some(confidence) = confidence {
        fields.push(format!("\"confidence\":{confidence}"));
    }
    if let Some(replaces) = replaces {
        fields.push(format!("\"replaces\":{}", quote(&replaces)));
    }
    let body = format!("{{{}}}", fields.join(","));
    let not_found = format!("not_found: no thread {thread} in this project");
    let response = agent.request_or_not_found(
        "POST",
        &format!(
            "/api/v1/w/{}/projects/{project}/threads/{thread}/posts",
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
    let answer = Value::parse(&answer).map_err(|_| {
        CliError::new("the server's answer about the post is not readable; is it an OmniState API?")
    })?;
    match answer.get("post").unwrap_or(&answer).str_at("id") {
        Some(id) => say(id),
        None => bail!("the server's answer has no post ID; try again"),
    }
    Ok(())
}
