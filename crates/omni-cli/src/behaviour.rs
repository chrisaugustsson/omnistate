//! `omni behaviour add`, `read`, `list`, `edit`, `move`, `ask`, `answer`, `question` and `questions`: the behaviour tree of a feature. A node is
//! named by its uuid, by its path (the slugs from the story down, joined by
//! `/`) or by `.` for the root. The project and the feature are found as for
//! `omni story add`.

use crate::args::Args;
use crate::commands::{Agent, agent, say};
use crate::error::{CliError, Result, bail};
use crate::json::{Value, quote};
use crate::projects::{creation_feature, is_uuid, project_id};

const ADD_USAGE: &str = "omni behaviour add --title <t> [--slug <s>] [--summary <text>] [--parent <node>] \
     [--same-as <node>] [--handoff <node>]";
const READ_USAGE: &str = "omni behaviour read <node>";
const EDIT_USAGE: &str = "omni behaviour edit <node> [--title <t>] [--slug <s>] [--summary <text>] \
     [--situation <text>] [--behaviour <text>] [--status open|done|out-of-scope] \
     [--same-as <node>] [--handoff <node>] [--grilled]";

const MOVE_USAGE: &str = "omni behaviour move <node> --to <parent>";
const ANSWER_USAGE: &str = "omni behaviour answer <question id> <answer> [--change] [--by human]";
const QUESTION_USAGE: &str = "omni behaviour question <question id>";
const ASK_USAGE: &str = "omni behaviour ask <node> <question>";

fn unreadable() -> CliError {
    CliError::new(
        "the server's answer about the behaviour tree is not readable; is it an OmniState API?",
    )
}

/// The behaviour path of the feature the command works in.
fn behaviour_path(args: &Args, agent: &Agent) -> Result<String> {
    let project = project_id(args, agent)?;
    let Some(feature) = creation_feature(args, agent, &project)? else {
        bail!("no feature: pass --feature <name or id>, or put feature = \"name\" in .omni.toml");
    };
    Ok(format!(
        "/api/v1/w/{}/projects/{project}/features/{feature}/behaviour",
        agent.workspace_id
    ))
}

/// The nodes of the feature, in tree order.
fn list_nodes(agent: &Agent, base: &str) -> Result<Vec<Value>> {
    let response = agent.request("GET", base, None)?;
    let body = Value::parse(&response.text()).map_err(|_| unreadable())?;
    Ok(body
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(unreadable)?
        .to_vec())
}

fn version_of(node: &Value) -> Result<i64> {
    match node.get("version") {
        Some(Value::Number(n)) => n.parse().map_err(|_| unreadable()),
        _ => Err(unreadable()),
    }
}

/// The id and the version of the node that `given` names: a uuid, a path, or
/// `.` for the root.
pub(crate) fn node_ref(agent: &Agent, base: &str, given: &str) -> Result<(String, i64)> {
    let nodes = list_nodes(agent, base)?;
    let path = if given == "." {
        ""
    } else {
        given.trim_matches('/')
    };
    let found = nodes.iter().find(|node| {
        if is_uuid(given) {
            node.str_at("id") == Some(given.to_ascii_lowercase().as_str())
        } else {
            node.str_at("path") == Some(path)
        }
    });
    let Some(found) = found else {
        bail!("not_found: no node {given} in this feature");
    };
    let id = found.str_at("id").ok_or_else(unreadable)?;
    Ok((id.to_string(), version_of(found)?))
}

/// `omni behaviour add --title <t> [--slug <s>] [--summary <text>] [--parent <node>]`
pub fn add(args: &Args) -> Result<()> {
    let Some(title) = &args.title else {
        bail!("this command needs a title: {ADD_USAGE}");
    };
    let agent = agent(args)?;
    let base = behaviour_path(args, &agent)?;
    let mut fields = vec![format!("\"title\":{}", quote(title))];
    if let Some(slug) = &args.slug {
        fields.push(format!("\"slug\":{}", quote(slug)));
    }
    if let Some(summary) = &args.summary {
        fields.push(format!("\"summary\":{}", quote(summary)));
    }
    if let Some(parent) = &args.parent {
        let (id, _) = node_ref(&agent, &base, parent)?;
        fields.push(format!("\"parent_id\":{}", quote(&id)));
    }
    for (name, given) in [("same_as", &args.same_as), ("handoff", &args.handoff)] {
        if let Some(given) = given {
            let (id, _) = node_ref(&agent, &base, given)?;
            fields.push(format!("\"{name}\":{}", quote(&id)));
        }
    }
    let body = format!("{{{}}}", fields.join(","));
    let response = agent.request("POST", &format!("{base}/nodes"), Some(body.as_bytes()))?;
    let text = response.text();
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    let answer = Value::parse(&text).map_err(|_| unreadable())?;
    match answer.get("node").unwrap_or(&answer).str_at("path") {
        Some(path) => say(path),
        None => bail!("the server's answer has no node path; try again"),
    }
    Ok(())
}

/// `omni behaviour read <node>`
pub fn read(args: &Args, node: Option<&str>) -> Result<()> {
    let Some(given) = node else {
        bail!("this command needs a node: {READ_USAGE}");
    };
    let agent = agent(args)?;
    let base = behaviour_path(args, &agent)?;
    let (id, _) = node_ref(&agent, &base, given)?;
    let response = agent.request("GET", &format!("{base}/nodes/{id}"), None)?;
    let text = response.text();
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    let answer = Value::parse(&text).map_err(|_| unreadable())?;
    let node = answer.get("node").ok_or_else(unreadable)?;
    let mut blocks: Vec<String> = Vec::new();
    for above in answer.get("chain").and_then(Value::as_array).unwrap_or(&[]) {
        blocks.push(read_block(above, above.get("questions")));
    }
    // The node's own questions sit beside it in the answer, not inside it.
    blocks.push(format!(
        "=== this node ===\n{}",
        read_block(node, answer.get("questions"))
    ));
    let mut out = blocks.join("\n\n");
    let mut sections: Vec<String> = Vec::new();
    let rules: Vec<String> = answer
        .get("rules")
        .and_then(Value::as_array)
        .unwrap_or(&[])
        .iter()
        .map(|rule| {
            let from = match rule.str_at("node_path") {
                Some("") | None => ".",
                Some(path) => path,
            };
            format!(
                "- {} ({}, from {from})",
                rule.str_at("text").unwrap_or_default(),
                rule.str_at("scope").unwrap_or("?")
            )
        })
        .collect();
    if !rules.is_empty() {
        sections.push(format!("Rules that apply here:\n{}", rules.join("\n")));
    }
    for (heading, key) in [("Children:", "children"), ("Siblings:", "siblings")] {
        let lines: Vec<String> = answer
            .get(key)
            .and_then(Value::as_array)
            .unwrap_or(&[])
            .iter()
            .map(|entry| {
                let title = entry.str_at("title").unwrap_or_default();
                let summary = entry.str_at("summary").filter(|s| !s.is_empty());
                format!(
                    "- {} — {} ({})",
                    entry.str_at("slug").unwrap_or(title),
                    summary.unwrap_or(title),
                    entry.str_at("status").unwrap_or("?")
                )
            })
            .collect();
        if !lines.is_empty() {
            sections.push(format!("{heading}\n{}", lines.join("\n")));
        }
    }
    for section in sections {
        out.push_str("\n\n");
        out.push_str(&section);
    }
    say(&out);
    Ok(())
}

/// One node as a block of the text read: title, path line, texts, decisions
/// and open questions, a line left out when its value is empty.
fn read_block(node: &Value, questions: Option<&Value>) -> String {
    let mut lines = vec![format!("# {}", node.str_at("title").unwrap_or_default())];
    let path = match node.str_at("path") {
        Some("") => ".",
        Some(path) => path,
        None => "?",
    };
    let level = match node.get("level") {
        Some(Value::Number(n)) if n == "0" => "feature",
        Some(Value::Number(n)) if n == "1" => "story",
        Some(Value::Number(_)) => "situation",
        _ => "?",
    };
    let mut head = format!(
        "path: {path}  level: {level}  status: {}",
        node.str_at("status").unwrap_or("?")
    );
    if let Some(Value::Bool(true)) = node.get("needs_pass") {
        head.push_str("  needs another pass");
    }
    for (word, key) in [("same-as", "same_as"), ("handoff", "handoff")] {
        if let Some(target) = node.get(key).and_then(|p| p.str_at("path")) {
            head.push_str(&format!("  {word}: {target}"));
        }
    }
    lines.push(head);
    for (label, key) in [
        ("Summary", "summary"),
        ("Situation", "situation"),
        ("Behaviour", "behaviour"),
    ] {
        if let Some(text) = node.str_at(key).filter(|t| !t.is_empty()) {
            lines.push(format!("{label}: {text}"));
        }
    }
    let questions = questions.and_then(Value::as_array).unwrap_or(&[]);
    let mut decisions = Vec::new();
    let mut open = Vec::new();
    for question in questions {
        let text = question.str_at("question").unwrap_or_default();
        match question.get("answer") {
            Some(current @ Value::Object(_)) => {
                let marker = if current.str_at("given_by") == Some("agent") {
                    " (default)"
                } else {
                    ""
                };
                decisions.push(format!(
                    "- {text} → {}{marker}",
                    current.str_at("answer").unwrap_or_default()
                ));
            }
            _ => open.push(format!(
                "- {text}  [{}]",
                question.str_at("id").unwrap_or_default()
            )),
        }
    }
    if !decisions.is_empty() {
        lines.push("Decisions:".to_string());
        lines.extend(decisions);
    }
    if !open.is_empty() {
        lines.push("Open questions:".to_string());
        lines.extend(open);
    }
    lines.join("\n")
}

/// `omni behaviour list [<node>]`
pub fn list(args: &Args, node: Option<&str>) -> Result<()> {
    let agent = agent(args)?;
    let base = behaviour_path(args, &agent)?;
    let mut query = Vec::new();
    if let Some(given) = node {
        query.push(format!("under={}", node_ref(&agent, &base, given)?.0));
    }
    if args.needs_pass {
        query.push("needs_pass=true".to_string());
    }
    let url = if query.is_empty() {
        base
    } else {
        format!("{base}?{}", query.join("&"))
    };
    let response = agent.request("GET", &url, None)?;
    let text = response.text();
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    let body = Value::parse(&text).map_err(|_| unreadable())?;
    let nodes = body
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(unreadable)?;
    let level_of = |node: &Value| match node.get("level") {
        Some(Value::Number(n)) => n.parse::<usize>().map_err(|_| unreadable()),
        _ => Err(unreadable()),
    };
    let Some(first) = nodes.first() else {
        return Ok(());
    };
    if args.needs_pass {
        for node in nodes {
            let path = match node.str_at("path") {
                Some("") => ".",
                Some(path) => path,
                None => "?",
            };
            let summary = node
                .str_at("summary")
                .filter(|s| !s.is_empty())
                .or(node.str_at("title"))
                .unwrap_or_default();
            say(&format!(
                "{path} — {summary} ({})",
                node.str_at("status").unwrap_or("?")
            ));
        }
        return Ok(());
    }
    let top = level_of(first)?;
    for node in nodes {
        let title = node.str_at("title").unwrap_or_default();
        let name = match node.str_at("slug") {
            Some(slug) if level_of(node)? > 0 => slug,
            _ => title,
        };
        let summary = node.str_at("summary").filter(|s| !s.is_empty());
        let marked = match node.get("needs_pass") {
            Some(Value::Bool(true)) => " [needs another pass]",
            _ => "",
        };
        let pointer = [("same-as", "same_as"), ("handoff", "handoff")]
            .iter()
            .find_map(|(word, key)| {
                let path = node.get(key)?.str_at("path")?;
                Some(format!(" → {word} {path}"))
            })
            .unwrap_or_default();
        say(&format!(
            "{}{name} — {} ({}){pointer}{marked}",
            "  ".repeat(level_of(node)?.saturating_sub(top)),
            summary.unwrap_or(title),
            node.str_at("status").unwrap_or("?")
        ));
    }
    Ok(())
}

/// `omni behaviour edit <node> [--title <t>] [--slug <s>] [--summary <text>]
/// [--situation <text>] [--behaviour <text>] [--status open|done|out-of-scope]`
pub fn edit(args: &Args, node: Option<&str>) -> Result<()> {
    let Some(given) = node else {
        bail!("this command needs a node: {EDIT_USAGE}");
    };
    let mut fields: Vec<String> = Vec::new();
    for (name, value) in [
        ("title", &args.title),
        ("slug", &args.slug),
        ("summary", &args.summary),
        ("situation", &args.situation),
        ("behaviour", &args.behaviour),
    ] {
        if let Some(value) = value {
            fields.push(format!("\"{name}\":{}", quote(value)));
        }
    }
    if let Some(status) = &args.status {
        let status = match status.as_str() {
            "open" | "done" => status.as_str(),
            "out-of-scope" | "out_of_scope" => "out_of_scope",
            other => bail!("{other:?} is not a status: use open, done or out-of-scope"),
        };
        fields.push(format!("\"status\":{}", quote(status)));
    }
    if args.grilled {
        fields.push("\"needs_pass\":false".to_string());
    }
    if fields.is_empty() && args.same_as.is_none() && args.handoff.is_none() {
        bail!("this command needs something to change: {EDIT_USAGE}");
    }
    let agent = agent(args)?;
    let base = behaviour_path(args, &agent)?;
    let (id, version) = node_ref(&agent, &base, given)?;
    for (name, pointer) in [("same_as", &args.same_as), ("handoff", &args.handoff)] {
        match pointer.as_deref() {
            Some("") => fields.push(format!("\"{name}\":null")),
            Some(given) => {
                let (target, _) = node_ref(&agent, &base, given)?;
                fields.push(format!("\"{name}\":{}", quote(&target)));
            }
            None => {}
        }
    }
    let body = format!("{{\"expected_version\":{version},{}}}", fields.join(","));
    let response =
        agent.request_conflicting("PATCH", &format!("{base}/nodes/{id}"), body.as_bytes())?;
    let text = response.text();
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    let answer = Value::parse(&text).map_err(|_| unreadable())?;
    let node = answer.get("node").unwrap_or(&answer);
    let path = match node.str_at("path") {
        Some("") => ".",
        Some(path) => path,
        None => bail!("the server's answer has no node path; try again"),
    };
    say(&format!("{path}  {}", node.str_at("status").unwrap_or("?")));
    Ok(())
}

/// `omni behaviour move <node> --to <parent>`
pub fn move_node(args: &Args, node: Option<&str>) -> Result<()> {
    let (Some(given), Some(to)) = (node, &args.to) else {
        bail!("this command needs a node and --to <parent>: {MOVE_USAGE}");
    };
    let agent = agent(args)?;
    let base = behaviour_path(args, &agent)?;
    let (id, version) = node_ref(&agent, &base, given)?;
    let (parent_id, _) = node_ref(&agent, &base, to)?;
    let body = format!(
        "{{\"expected_version\":{version},\"parent_id\":{}}}",
        quote(&parent_id)
    );
    let response =
        agent.request_conflicting("POST", &format!("{base}/nodes/{id}/move"), body.as_bytes())?;
    let text = response.text();
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    let answer = Value::parse(&text).map_err(|_| unreadable())?;
    let node = answer.get("node").unwrap_or(&answer);
    match node.str_at("path") {
        Some(path) => say(path),
        None => bail!("the server's answer has no node path; try again"),
    }
    Ok(())
}

/// `omni behaviour ask <node> <question>`
pub fn ask(args: &Args, node: Option<&str>, question: Option<&str>) -> Result<()> {
    let (Some(given), Some(question)) = (node, question) else {
        bail!("this command needs a node and a question: {ASK_USAGE}");
    };
    let agent = agent(args)?;
    let base = behaviour_path(args, &agent)?;
    let (id, _) = node_ref(&agent, &base, given)?;
    let mut body = format!("{{\"question\":{}", quote(question));
    if let Some(answer) = &args.answer {
        body.push_str(&format!(",\"answer\":{}", quote(answer)));
    }
    if let Some(by) = &args.by {
        body.push_str(&format!(",\"by\":{}", quote(by)));
    }
    body.push('}');
    let response = agent.request(
        "POST",
        &format!("{base}/nodes/{id}/questions"),
        Some(body.as_bytes()),
    )?;
    let text = response.text();
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    let answer = Value::parse(&text).map_err(|_| unreadable())?;
    match answer.get("question").unwrap_or(&answer).str_at("id") {
        Some(id) => say(id),
        None => bail!("the server's answer has no question id; try again"),
    }
    Ok(())
}

/// `omni behaviour questions [<node>] [--open]`
pub fn questions(args: &Args, node: Option<&str>) -> Result<()> {
    let agent = agent(args)?;
    let base = behaviour_path(args, &agent)?;
    let mut query = Vec::new();
    if args.default_answers {
        query.push("state=answered".to_string());
        query.push("by=agent".to_string());
    } else if args.open {
        query.push("state=open".to_string());
    }
    if let Some(given) = node {
        query.push(format!("under={}", node_ref(&agent, &base, given)?.0));
    }
    let mut url = format!("{base}/questions");
    if !query.is_empty() {
        url = format!("{url}?{}", query.join("&"));
    }
    let response = agent.request("GET", &url, None)?;
    let text = response.text();
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    let body = Value::parse(&text).map_err(|_| unreadable())?;
    let questions = body
        .get("questions")
        .and_then(Value::as_array)
        .ok_or_else(unreadable)?;
    for question in questions {
        let path = match question.str_at("node_path") {
            Some("") => ".",
            Some(path) => path,
            None => "?",
        };
        let mut line = format!(
            "{path}  {}  {}",
            question.str_at("id").unwrap_or("?"),
            question.str_at("question").unwrap_or_default()
        );
        if args.default_answers
            && let Some(answer) = question.get("answer").and_then(|a| a.str_at("answer"))
        {
            line.push_str(&format!(" → {answer}"));
        }
        say(&line);
    }
    Ok(())
}

/// `omni behaviour answer <question id> <answer> [--by human]`
pub fn answer(args: &Args, question: Option<&str>, answer: Option<&str>) -> Result<()> {
    let (Some(given), Some(answer)) = (question, answer) else {
        bail!("this command needs a question ID and an answer: {ANSWER_USAGE}");
    };
    let agent = agent(args)?;
    let base = behaviour_path(args, &agent)?;
    let response = agent.request("GET", &format!("{base}/questions"), None)?;
    let listed = Value::parse(&response.text()).map_err(|_| unreadable())?;
    let questions = listed
        .get("questions")
        .and_then(Value::as_array)
        .ok_or_else(unreadable)?;
    let Some(found) = questions
        .iter()
        .find(|question| question.str_at("id") == Some(given))
    else {
        bail!("not_found: no question {given} in this feature");
    };
    let version = version_of(found)?;
    let mut body = format!(
        "{{\"expected_version\":{version},\"answer\":{}",
        quote(answer)
    );
    if let Some(by) = &args.by {
        body.push_str(&format!(",\"by\":{}", quote(by)));
    }
    if args.change {
        body.push_str(",\"change\":true");
    }
    body.push('}');
    let response = agent
        .request_conflicting(
            "POST",
            &format!("{base}/questions/{given}/answers"),
            body.as_bytes(),
        )
        .map_err(refused_not_stale)?;
    let text = response.text();
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    say(given);
    Ok(())
}

/// Exit 3 is for a stale version only: any other 409 code (`already_answered`,
/// `not_answered`, `answered_by_human`) ends with exit 1 and the API's code.
fn refused_not_stale(error: CliError) -> CliError {
    let message = error.to_string();
    match message
        .strip_prefix("conflict: ")
        .and_then(|rest| rest.strip_suffix(" ; read it again"))
    {
        Some(code) if code != "conflict" => {
            CliError::new(format!("{code}: the server answered HTTP 409"))
        }
        _ => error,
    }
}

/// Who gave an answer: `person`, or `agent <run>/<role>`.
fn giver(answer: &Value) -> String {
    if answer.str_at("given_by") == Some("human") {
        return "person".to_string();
    }
    let author = answer.get("author");
    format!(
        "agent {}/{}",
        author.and_then(|a| a.str_at("run")).unwrap_or("?"),
        author.and_then(|a| a.str_at("role")).unwrap_or("?")
    )
}

/// `omni behaviour question <question id>`
pub fn question(args: &Args, question: Option<&str>) -> Result<()> {
    let Some(given) = question else {
        bail!("this command needs a question ID: {QUESTION_USAGE}");
    };
    let agent = agent(args)?;
    let base = behaviour_path(args, &agent)?;
    let response = agent.request("GET", &format!("{base}/questions/{given}"), None)?;
    let text = response.text();
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    let body = Value::parse(&text).map_err(|_| unreadable())?;
    let question = body.get("question").ok_or_else(unreadable)?;
    let path = match question.str_at("node_path") {
        Some("") => ".",
        Some(path) => path,
        None => "?",
    };
    say(&format!(
        "{path}  {}  {}",
        question.str_at("id").unwrap_or("?"),
        question.str_at("question").unwrap_or_default()
    ));
    let answers = question
        .get("answers")
        .and_then(Value::as_array)
        .ok_or_else(unreadable)?;
    let line = |answer: &Value| {
        let default = if answer.str_at("given_by") == Some("agent") {
            " (default)"
        } else {
            ""
        };
        format!(
            "{} — {}{default}, {}",
            answer.str_at("answer").unwrap_or_default(),
            giver(answer),
            answer.str_at("at").unwrap_or("?")
        )
    };
    if let Some((current, earlier)) = answers.split_last() {
        say(&format!("answer: {}", line(current)));
        for answer in earlier {
            say(&format!("earlier: {}", line(answer)));
        }
    }
    Ok(())
}

const RULE_ADD_USAGE: &str = "omni behaviour rule add <node> <text> [--scope node|story|feature]";
const RULE_EDIT_USAGE: &str =
    "omni behaviour rule edit <rule id> [--text <t>] [--scope <s>] [--withdraw]";

/// `omni behaviour rule add <node> <text> [--scope node|story|feature]`
pub fn rule_add(args: &Args, node: Option<&str>, text: Option<&str>) -> Result<()> {
    let (Some(given), Some(text)) = (node, text) else {
        bail!("this command needs a node and a text: {RULE_ADD_USAGE}");
    };
    let agent = agent(args)?;
    let base = behaviour_path(args, &agent)?;
    let (id, _) = node_ref(&agent, &base, given)?;
    let mut body = format!("{{\"text\":{}", quote(text));
    if let Some(scope) = &args.scope {
        body.push_str(&format!(",\"scope\":{}", quote(scope)));
    }
    body.push('}');
    let response = agent.request(
        "POST",
        &format!("{base}/nodes/{id}/rules"),
        Some(body.as_bytes()),
    )?;
    let text = response.text();
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    let answer = Value::parse(&text).map_err(|_| unreadable())?;
    match answer.get("rule").unwrap_or(&answer).str_at("id") {
        Some(id) => say(id),
        None => bail!("the server's answer has no rule id; try again"),
    }
    Ok(())
}

/// The rules of the feature as the API lists them, withdrawn ones included.
fn listed_rules(agent: &Agent, url: &str) -> Result<Vec<Value>> {
    let response = agent.request("GET", url, None)?;
    let body = Value::parse(&response.text()).map_err(|_| unreadable())?;
    Ok(body
        .get("rules")
        .and_then(Value::as_array)
        .ok_or_else(unreadable)?
        .to_vec())
}

/// `omni behaviour rule edit <rule id> [--text <t>] [--scope <s>] [--withdraw]`
pub fn rule_edit(args: &Args, rule: Option<&str>) -> Result<()> {
    let Some(given) = rule else {
        bail!("this command needs a rule ID: {RULE_EDIT_USAGE}");
    };
    if args.text.is_none() && args.scope.is_none() && !args.withdraw {
        bail!("nothing to change: {RULE_EDIT_USAGE}");
    }
    let agent = agent(args)?;
    let base = behaviour_path(args, &agent)?;
    let rules = listed_rules(&agent, &format!("{base}/rules"))?;
    let Some(found) = rules.iter().find(|rule| rule.str_at("id") == Some(given)) else {
        bail!("not_found: no rule {given} in this feature");
    };
    let mut body = format!("{{\"expected_version\":{}", version_of(found)?);
    if let Some(text) = &args.text {
        body.push_str(&format!(",\"text\":{}", quote(text)));
    }
    if let Some(scope) = &args.scope {
        body.push_str(&format!(",\"scope\":{}", quote(scope)));
    }
    if args.withdraw {
        body.push_str(",\"withdrawn\":true");
    }
    body.push('}');
    let response =
        agent.request_conflicting("PATCH", &format!("{base}/rules/{given}"), body.as_bytes())?;
    let text = response.text();
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    say(given);
    Ok(())
}

/// `omni behaviour rules [<node>] [--scope <s>]`
pub fn rules(args: &Args, node: Option<&str>) -> Result<()> {
    let agent = agent(args)?;
    let base = behaviour_path(args, &agent)?;
    let mut query = Vec::new();
    if let Some(scope) = &args.scope {
        query.push(format!("scope={scope}"));
    }
    if let Some(given) = node {
        query.push(format!("under={}", node_ref(&agent, &base, given)?.0));
    }
    let mut url = format!("{base}/rules");
    if !query.is_empty() {
        url = format!("{url}?{}", query.join("&"));
    }
    let response = agent.request("GET", &url, None)?;
    let text = response.text();
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    let body = Value::parse(&text).map_err(|_| unreadable())?;
    let rules = body
        .get("rules")
        .and_then(Value::as_array)
        .ok_or_else(unreadable)?;
    for rule in rules {
        if matches!(rule.get("withdrawn"), Some(Value::Bool(true))) {
            continue;
        }
        let path = match rule.str_at("node_path") {
            Some("") => ".",
            Some(path) => path,
            None => "?",
        };
        say(&format!(
            "{}  {path}  {}  {}",
            rule.str_at("scope").unwrap_or("?"),
            rule.str_at("id").unwrap_or("?"),
            rule.str_at("text").unwrap_or_default()
        ));
    }
    Ok(())
}
