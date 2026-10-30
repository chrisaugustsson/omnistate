//! `omni post <thread>`: posts to a thread, a message with a one-line summary
//! and a Markdown body. The body is the argument after the thread, else
//! `--body-file`, else standard input; each `--attach` file is uploaded first,
//! and everything that can be checked here is checked before anything is sent.
//! The project is found as for `omni project`.

use crate::args::Args;
use crate::attach;
use crate::commands::{agent, read_body, say};
use crate::error::{CliError, Result, bail};
use crate::json::{Value, quote};
use crate::projects::{is_uuid, project_id};
use std::io::IsTerminal;
use std::path::Path;

const USAGE: &str = "omni post <thread> --summary <text> [<body>] [--body-file <path>] \
     [--attach <file>]... [--link <target>]... [--confidence <0-1>] [--replaces <post>]";
const MAX_SUMMARY_CHARS: usize = 200;
const SHOW_USAGE: &str = "omni post show <post>";

/// The role of a post's author: `human` for the human's posts.
pub(crate) fn role(post: &Value) -> &str {
    let author = post.get("author");
    match author.and_then(|author| author.str_at("kind")) {
        Some("human") => "human",
        _ => author
            .and_then(|author| author.str_at("role"))
            .unwrap_or("agent"),
    }
}

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

/// The text of the post: the argument, else `--body-file`, or else all of
/// standard input.
fn body(args: &Args, given: Option<&str>) -> Result<String> {
    if let Some(given) = given {
        if args.body_file.is_some() {
            bail!("give the body as an argument or with --body-file, not both: {USAGE}");
        }
        return Ok(given.to_string());
    }
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

/// `omni post <thread> [<body>] ...`
pub fn run(args: &Args, thread: Option<&str>, given_body: Option<&str>) -> Result<()> {
    let Some(thread) = thread else {
        bail!("this command needs the thread's ID: {USAGE}");
    };
    if !is_uuid(thread) {
        bail!("{thread:?} is not a thread ID: {USAGE}");
    }
    let thread = thread.to_ascii_lowercase();
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
    let text = body(args, given_body)?;
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

/// `omni post show <post>`: prints one whole post, the header line, who and
/// where it is from, and the Markdown body. A post that the caller may not
/// read yet is as unknown as one that does not exist.
pub fn show(args: &Args, post: Option<&str>) -> Result<()> {
    let Some(post) = post else {
        bail!("this command needs the post's ID: {SHOW_USAGE}");
    };
    if !is_uuid(post) {
        bail!("{post:?} is not a post ID: {SHOW_USAGE}");
    }
    let post = post.to_ascii_lowercase();

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let not_found = format!("not_found: no post {post} that you can read in this project");
    let response = agent.request_or_not_found(
        "GET",
        &format!(
            "/api/v1/w/{}/projects/{project}/posts/{post}",
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
    let answer = Value::parse(&text).map_err(|_| {
        CliError::new("the server's answer about the post is not readable; is it an OmniState API?")
    })?;
    let shown = answer.get("post").unwrap_or(&answer);
    say(&format!(
        "{}  {}",
        role(shown),
        shown.str_at("summary").unwrap_or_default()
    ));
    say(&format!("id: {}", shown.str_at("id").unwrap_or("?")));
    if let Some(Value::Number(n)) = shown.get("confidence") {
        say(&format!("confidence: {n}"));
    }
    if let Some(replaces) = shown.str_at("replaces") {
        say(&format!("replaces: {replaces}"));
    }
    for field in ["links", "attachments"] {
        for item in shown
            .get(field)
            .and_then(Value::as_array)
            .unwrap_or_default()
        {
            say(&format!("{}: {item}", field.trim_end_matches('s')));
        }
    }
    say("");
    say(shown.str_at("body").unwrap_or_default());
    Ok(())
}
