//! `omni story add`, `list` and `read`: stories, the work items of a feature.
//! `add` saves one story written in the prd format; the server numbers it.
//! The project and the feature are found as for `omni thread create`.

use crate::args::Args;
use crate::commands::{Agent, agent, say};
use crate::error::{CliError, Result, bail};
use crate::json::{Value, quote};
use crate::projects::{creation_feature, is_uuid, project_id};

const ADD_USAGE: &str =
    "omni story add <file> [--priority <n>] [--link <doc>:<node>[:<label>]]... [--number-from <n>]";
const LIST_USAGE: &str = "omni story list [--status <status>] [--pickable]";
const READ_USAGE: &str = "omni story read <key|id>";

fn unreadable() -> CliError {
    CliError::new("the server's answer about the stories is not readable; is it an OmniState API?")
}

/// The stories path of the feature the command works in.
fn stories_path(args: &Args, agent: &Agent) -> Result<String> {
    let project = project_id(args, agent)?;
    let Some(feature) = creation_feature(args, agent, &project)? else {
        bail!("no feature: pass --feature <name or id>, or put feature = \"name\" in .omni.toml");
    };
    Ok(format!(
        "/api/v1/w/{}/projects/{project}/features/{feature}/stories",
        agent.workspace_id
    ))
}

fn list_stories(agent: &Agent, path: &str) -> Result<Vec<Value>> {
    let response = agent.request("GET", path, None)?;
    let body = Value::parse(&response.text()).map_err(|_| unreadable())?;
    Ok(body
        .get("stories")
        .and_then(Value::as_array)
        .ok_or_else(unreadable)?
        .to_vec())
}

/// The text after a bold label such as `**Topic:**` at the start of a line.
fn labelled<'a>(line: &'a str, label: &str) -> Option<&'a str> {
    line.trim_start()
        .strip_prefix(&format!("**{label}:**"))
        .map(str::trim)
}

fn is_label(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("**") && line.contains(":**")
}

/// The lines of the block that starts at line `start`: the line and the lines
/// after it up to a blank line, another label, a criterion or a heading.
fn block(lines: &[&str], start: usize) -> Vec<String> {
    let mut block = vec![lines[start].trim_end().to_string()];
    for line in &lines[start + 1..] {
        let trimmed = line.trim_start();
        if trimmed.is_empty()
            || is_label(trimmed)
            || trimmed.starts_with("- [")
            || trimmed.starts_with('#')
        {
            break;
        }
        block.push(line.trim_end().to_string());
    }
    block
}

/// The backticked paths of the `change:` segment of a Files line, without
/// anything in parentheses.
fn changed_files(files: &str) -> Vec<String> {
    let Some(at) = files.find("change:") else {
        return Vec::new();
    };
    let rest = &files[at + "change:".len()..];
    let end = [rest.find(';'), rest.find(" · ")]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(rest.len());
    let mut paths = Vec::new();
    let (mut depth, mut open) = (0usize, None);
    for (i, c) in rest[..end].char_indices() {
        match (c, open) {
            ('`', None) if depth == 0 => open = Some(i + 1),
            ('`', Some(from)) => {
                paths.push(rest[from..i].to_string());
                open = None;
            }
            ('(', None) => depth += 1,
            (')', None) => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    paths
}

fn story_key(word: &str) -> Option<u64> {
    let digits = word
        .strip_prefix("US-")
        .or_else(|| word.strip_prefix("us-"))?;
    digits.parse().ok()
}

/// The keys named on a `Blocked by:` line.
fn blocker_keys(value: &str) -> Vec<String> {
    value
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
        .filter_map(story_key)
        .map(|n| format!("US-{n}"))
        .collect()
}

struct Parsed {
    title: String,
    text: String,
    topic: String,
    blockers: Vec<String>,
    files: Vec<String>,
    criteria: Vec<(String, bool)>,
}

fn parse_story(source: &str) -> Result<Parsed> {
    let lines: Vec<&str> = source.lines().collect();
    let heading = lines
        .iter()
        .find_map(|line| line.trim_start().strip_prefix("### "))
        .ok_or_else(|| CliError::new("the file has no story heading `### US-<n>: <title>`"))?
        .trim();
    let title = match heading.strip_prefix("US-") {
        Some(rest) => match rest.split_once(':') {
            Some((n, title)) if n.chars().all(|c| c.is_ascii_digit()) && !n.is_empty() => {
                title.trim()
            }
            _ => heading,
        },
        None => heading,
    };
    if title.is_empty() {
        bail!("the story's title is blank");
    }

    let mut description = None;
    let mut kept = Vec::new();
    let (mut topic, mut blocked, mut files) = (String::new(), String::new(), String::new());
    let mut criteria = Vec::new();
    let mut in_criteria = false;
    for (i, line) in lines.iter().enumerate() {
        if let Some(rest) = labelled(line, "Description") {
            let mut paragraph = block(&lines, i);
            paragraph[0] = rest.to_string();
            description = Some(paragraph.join("\n"));
        } else if let Some(rest) = labelled(line, "Topic") {
            topic = rest.to_string();
            kept.push(block(&lines, i).join("\n"));
        } else if labelled(line, "Interfaces").is_some() {
            kept.push(block(&lines, i).join("\n"));
        } else if let Some(rest) = labelled(line, "Files") {
            files = rest.to_string();
            kept.push(block(&lines, i).join("\n"));
        } else if let Some(rest) = labelled(line, "Blocked by") {
            blocked = rest.to_string();
        }
        if labelled(line, "Acceptance Criteria").is_some() {
            in_criteria = true;
        } else if in_criteria && let Some(text) = line.trim_start().strip_prefix("- [ ] ") {
            let text = text.trim_end();
            let human = text.ends_with("Verify in browser using dev-browser skill")
                || text.contains("(human check)");
            criteria.push((text.to_string(), human));
        }
    }
    let Some(description) = description else {
        bail!("the story has no **Description:** paragraph");
    };
    let mut text = description;
    if !kept.is_empty() {
        text.push_str("\n\n");
        text.push_str(&kept.join("\n"));
    }
    Ok(Parsed {
        title: title.to_string(),
        text,
        topic,
        blockers: blocker_keys(&blocked),
        files: changed_files(&files),
        criteria,
    })
}

/// The `--link` value as the JSON object the API takes.
fn link(target: &str) -> Result<String> {
    let mut parts = target.splitn(3, ':');
    let (doc, node, label) = (
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
    );
    if !is_uuid(doc) || node.is_empty() {
        bail!("{target:?} is not a link for --link; use <doc>:<node>[:<label>] with the doc's ID");
    }
    Ok(format!(
        "{{\"doc_id\":{},\"node_id\":{},\"label\":{}}}",
        quote(&doc.to_ascii_lowercase()),
        quote(node),
        quote(label)
    ))
}

fn count(value: &str, name: &str) -> Result<i64> {
    value
        .trim()
        .parse()
        .map_err(|_| CliError::new(format!("{name} {value:?} is not a number: {ADD_USAGE}")))
}

/// `omni story add <file> [--priority <n>] [--link <doc>:<node>[:<label>]]... [--number-from <n>]`
pub fn add(args: &Args, file: Option<&str>) -> Result<()> {
    let Some(file) = file else {
        bail!("this command needs the story's file: {ADD_USAGE}");
    };
    let source = std::fs::read_to_string(file)
        .map_err(|e| CliError::new(format!("cannot read {file}: {e}")))?;
    let story = parse_story(&source)?;
    let priority = args
        .priority
        .as_deref()
        .map(|p| count(p, "--priority"))
        .transpose()?;
    let number_from = args
        .number_from
        .as_deref()
        .map(|n| count(n, "--number-from"))
        .transpose()?;
    let links = args
        .link
        .iter()
        .map(|target| link(target))
        .collect::<Result<Vec<_>>>()?;

    let agent = agent(args)?;
    let path = stories_path(args, &agent)?;
    let mut blocked_by = Vec::new();
    if !story.blockers.is_empty() {
        let existing = list_stories(&agent, &path)?;
        for key in &story.blockers {
            let id = existing
                .iter()
                .find(|s| s.str_at("key") == Some(key.as_str()))
                .and_then(|s| s.str_at("id"))
                .ok_or_else(|| {
                    CliError::new(format!("not_found: no story {key} in this feature"))
                })?;
            blocked_by.push(quote(id));
        }
    }
    let criteria: Vec<String> = story
        .criteria
        .iter()
        .map(|(text, human)| format!("{{\"text\":{},\"human_check\":{human}}}", quote(text)))
        .collect();
    let files: Vec<String> = story.files.iter().map(|f| quote(f)).collect();
    let mut fields = vec![
        format!("\"title\":{}", quote(&story.title)),
        format!("\"text\":{}", quote(&story.text)),
        format!("\"topic\":{}", quote(&story.topic)),
        format!("\"criteria\":[{}]", criteria.join(",")),
        format!("\"blocked_by\":[{}]", blocked_by.join(",")),
        format!("\"files_planned\":[{}]", files.join(",")),
        format!("\"links\":[{}]", links.join(",")),
    ];
    if let Some(priority) = priority {
        fields.push(format!("\"priority\":{priority}"));
    }
    if let Some(number_from) = number_from {
        fields.push(format!("\"number_from\":{number_from}"));
    }
    let body = format!("{{{}}}", fields.join(","));
    let response = agent.request("POST", &path, Some(body.as_bytes()))?;
    let text = response.text();
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    let answer = Value::parse(&text).map_err(|_| unreadable())?;
    match answer.get("story").unwrap_or(&answer).str_at("key") {
        Some(key) => say(key),
        None => bail!("the server's answer has no story key; try again"),
    }
    Ok(())
}

/// `omni story list [--status <status>] [--pickable]`
pub fn list(args: &Args) -> Result<()> {
    let agent = agent(args)?;
    let mut path = stories_path(args, &agent)?;
    let mut query = Vec::new();
    if let Some(status) = &args.status {
        if status.is_empty() || !status.chars().all(|c| c.is_ascii_alphabetic()) {
            bail!("{status:?} is not a status: {LIST_USAGE}");
        }
        query.push(format!("status={status}"));
    }
    if args.pickable {
        query.push("pickable=true".to_string());
    }
    if !query.is_empty() {
        path = format!("{path}?{}", query.join("&"));
    }
    if args.json {
        let response = agent.request("GET", &path, None)?;
        say(response.text().trim_end());
        return Ok(());
    }
    for story in list_stories(&agent, &path)? {
        say(&format!(
            "{}  P{}  {}  {}",
            story.str_at("key").unwrap_or("?"),
            story
                .get("priority")
                .map(|p| p.to_string())
                .unwrap_or("?".into()),
            story.str_at("status").unwrap_or("?"),
            story.str_at("title").unwrap_or_default()
        ));
    }
    Ok(())
}

/// `omni story read <key|id>`
pub fn read(args: &Args, story: Option<&str>) -> Result<()> {
    let Some(given) = story else {
        bail!("this command needs the story's key or ID: {READ_USAGE}");
    };
    let key = story_key(given).map(|n| format!("US-{n}"));
    if key.is_none() && !is_uuid(given) {
        bail!("{given:?} is not a story key or ID: {READ_USAGE}");
    }
    let agent = agent(args)?;
    let path = stories_path(args, &agent)?;
    let all = list_stories(&agent, &path)?;
    let found = all.iter().find(|s| match &key {
        Some(key) => s.str_at("key") == Some(key.as_str()),
        None => s.str_at("id") == Some(given.to_ascii_lowercase().as_str()),
    });
    let Some(found) = found else {
        bail!("not_found: no story {given} in this feature");
    };
    if args.json {
        say(&found.to_string());
        return Ok(());
    }
    say(found.str_at("text").unwrap_or_default());
    say("");
    for criterion in found
        .get("criteria")
        .and_then(Value::as_array)
        .unwrap_or_default()
    {
        let ticked = matches!(criterion.get("ticked"), Some(Value::Bool(true)));
        let pending = matches!(criterion.get("pending"), Some(Value::Bool(true)));
        say(&format!(
            "[{}] {}{}",
            if ticked { "x" } else { " " },
            criterion.str_at("text").unwrap_or_default(),
            if pending { " (pending)" } else { "" }
        ));
    }
    let blockers: Vec<&str> = found
        .get("blocked_by")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
        .map(|id| {
            all.iter()
                .find(|s| s.str_at("id") == Some(id))
                .and_then(|s| s.str_at("key"))
                .unwrap_or(id)
        })
        .collect();
    if !blockers.is_empty() {
        say(&format!("blocked by: {}", blockers.join(", ")));
    }
    if let Some(holder) = found.get("holder")
        && !matches!(holder, Value::Null)
    {
        say(&format!(
            "holder: {} {} {}",
            holder.str_at("run").unwrap_or("?"),
            holder.str_at("stage").unwrap_or("?"),
            holder.str_at("at").unwrap_or("?")
        ));
    }
    for finding in found
        .get("findings")
        .and_then(Value::as_array)
        .unwrap_or_default()
    {
        if matches!(finding.get("resolved"), Some(Value::Bool(true))) {
            continue;
        }
        say(&format!(
            "[{}] {}",
            finding.str_at("stage").unwrap_or("?"),
            finding.str_at("text").unwrap_or_default()
        ));
    }
    Ok(())
}

const WRITE_USAGE: &str = "omni story edit|cancel|claim|release|move|block|unblock|tick|finding|resolve|decision|files <key> ...; see `omni --help`";

/// The option's value, or an error naming the option and the verb.
fn needs<'a>(value: &'a Option<String>, name: &str, verb: &str) -> Result<&'a str> {
    match value.as_deref() {
        Some(value) => Ok(value),
        None => bail!("omni story {verb} needs {name}; see `omni --help`"),
    }
}

fn index(value: &str, name: &str) -> Result<u64> {
    value
        .trim()
        .parse()
        .map_err(|_| CliError::new(format!("{name} {value:?} is not a number: {WRITE_USAGE}")))
}

/// The story records of `omni story edit|cancel|claim|release|move|block|
/// unblock|tick|finding|resolve|decision|files`: the story is read first and
/// its version goes along as `expected_version`.
pub fn write(args: &Args, verb: &str, rest: &[&str]) -> Result<()> {
    let (given, paths) = match rest {
        [given, paths @ ..] => (*given, paths),
        [] => bail!("this command needs the story's key or ID: {WRITE_USAGE}"),
    };
    if verb != "files" && !paths.is_empty() {
        bail!("omni story {verb} takes only the story's key or ID; see `omni --help`");
    }
    let key = story_key(given).map(|n| format!("US-{n}"));
    if key.is_none() && !is_uuid(given) {
        bail!("{given:?} is not a story key or ID: {WRITE_USAGE}");
    }
    let run = || needs(&args.run, "--run <id>", verb);
    // Everything that can be wrong in the options is found before the server
    // is asked anything.
    let mut fields: Vec<String> = Vec::new();
    let (action, method) = match verb {
        "edit" => {
            if let Some(priority) = &args.priority {
                fields.push(format!("\"priority\":{}", count(priority, "--priority")?));
            }
            if let Some(text) = &args.text {
                let text = std::fs::read_to_string(text)
                    .map_err(|e| CliError::new(format!("cannot read {text}: {e}")))?;
                fields.push(format!("\"text\":{}", quote(&text)));
            }
            if !args.link.is_empty() {
                let links = args
                    .link
                    .iter()
                    .map(|target| link(target))
                    .collect::<Result<Vec<_>>>()?;
                fields.push(format!("\"links\":[{}]", links.join(",")));
            }
            if args.blocked_by.is_none() && fields.is_empty() {
                bail!("omni story edit needs --priority, --blocked-by, --text or --link");
            }
            ("", "PATCH")
        }
        "cancel" => ("/cancel", "POST"),
        "claim" => {
            fields.push(format!("\"run\":{}", quote(run()?)));
            fields.push(format!(
                "\"stage\":{}",
                quote(needs(&args.stage, "--stage <stage>", verb)?)
            ));
            ("/claim", "POST")
        }
        "release" | "block" | "unblock" => {
            fields.push(format!("\"run\":{}", quote(run()?)));
            (
                match verb {
                    "release" => "/release",
                    "block" => "/block",
                    _ => "/unblock",
                },
                "POST",
            )
        }
        "move" => {
            fields.push(format!("\"run\":{}", quote(run()?)));
            fields.push(format!(
                "\"to\":{}",
                quote(needs(&args.to, "--to <column>", verb)?)
            ));
            ("/move", "POST")
        }
        "tick" => {
            fields.push(format!("\"run\":{}", quote(run()?)));
            let criterion = index(
                needs(&args.criterion, "--criterion <n>", verb)?,
                "--criterion",
            )?;
            fields.push(format!("\"criterion\":{criterion}"));
            fields.push(format!("\"ticked\":{}", !args.unticked));
            if args.pending {
                fields.push("\"pending\":true".to_string());
            }
            ("/tick", "POST")
        }
        "finding" => {
            fields.push(format!("\"run\":{}", quote(run()?)));
            fields.push(format!(
                "\"stage\":{}",
                quote(needs(&args.stage, "--stage <stage>", verb)?)
            ));
            fields.push(format!(
                "\"text\":{}",
                quote(needs(&args.text, "--text <text>", verb)?)
            ));
            let files: Vec<String> = args.file.iter().map(|f| quote(f)).collect();
            fields.push(format!("\"files\":[{}]", files.join(",")));
            ("/findings", "POST")
        }
        "resolve" => {
            fields.push(format!("\"run\":{}", quote(run()?)));
            ("", "POST")
        }
        "decision" => {
            fields.push(format!("\"run\":{}", quote(run()?)));
            for (value, name) in [
                (&args.question, "question"),
                (&args.choice, "choice"),
                (&args.by, "by"),
            ] {
                let flag = format!("--{name} <{name}>");
                fields.push(format!("\"{name}\":{}", quote(needs(value, &flag, verb)?)));
            }
            ("/decisions", "POST")
        }
        _ => {
            fields.push(format!("\"run\":{}", quote(run()?)));
            let files: Vec<String> = paths.iter().map(|p| quote(p)).collect();
            fields.push(format!("\"files_changed\":[{}]", files.join(",")));
            ("/files", "POST")
        }
    };
    let finding = if verb == "resolve" {
        Some(index(
            needs(&args.finding, "--finding <n>", verb)?,
            "--finding",
        )?)
    } else {
        None
    };

    let agent = agent(args)?;
    let path = stories_path(args, &agent)?;
    let all = list_stories(&agent, &path)?;
    let found = all.iter().find(|s| match &key {
        Some(key) => s.str_at("key") == Some(key.as_str()),
        None => s.str_at("id") == Some(given.to_ascii_lowercase().as_str()),
    });
    let Some(found) = found else {
        bail!("not_found: no story {given} in this feature");
    };
    let (Some(id), Some(Value::Number(version))) = (found.str_at("id"), found.get("version"))
    else {
        return Err(unreadable());
    };
    if verb == "edit"
        && let Some(keys) = &args.blocked_by
    {
        let mut ids = Vec::new();
        for word in keys.split(',').map(str::trim).filter(|w| !w.is_empty()) {
            let Some(n) = story_key(word) else {
                bail!("{word:?} is not a story key for --blocked-by; use US-1,US-2");
            };
            let wanted = format!("US-{n}");
            let id = all
                .iter()
                .find(|s| s.str_at("key") == Some(wanted.as_str()))
                .and_then(|s| s.str_at("id"))
                .ok_or_else(|| {
                    CliError::new(format!("not_found: no story {wanted} in this feature"))
                })?;
            ids.push(quote(id));
        }
        fields.push(format!("\"blocked_by\":[{}]", ids.join(",")));
    }
    fields.insert(0, format!("\"expected_version\":{version}"));
    let action = match finding {
        Some(n) => format!("/findings/{n}/resolve"),
        None => action.to_string(),
    };
    let body = format!("{{{}}}", fields.join(","));
    let response =
        agent.request_conflicting(method, &format!("{path}/{id}{action}"), body.as_bytes())?;
    let answer = Value::parse(&response.text()).map_err(|_| unreadable())?;
    let story = answer.get("story").unwrap_or(&answer);
    if args.json {
        say(&story.to_string());
    } else {
        say(&format!(
            "{}  {}",
            story.str_at("key").unwrap_or("?"),
            story.str_at("status").unwrap_or("?")
        ));
    }
    Ok(())
}
