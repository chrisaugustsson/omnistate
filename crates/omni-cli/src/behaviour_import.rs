//! `omni behaviour import <folder> [--check]`: reads a grill-tree folder (a
//! `node.md` in every node folder) into the empty behaviour tree of a feature
//! with one request. Everything is read and checked before anything is sent.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::args::Args;
use crate::behaviour::behaviour_path;
use crate::commands::{agent, say};
use crate::error::{CliError, Result, bail};
use crate::json::{Value, quote};

const USAGE: &str = "omni behaviour import <folder> [--check]";

struct Node {
    /// The folder names from the root down, joined by `/`; empty for the root.
    path: String,
    slug: String,
    title: String,
    summary: String,
    situation: String,
    behaviour: String,
    status: String,
    same_as: String,
    handoff: String,
    /// The children in order, as paths.
    children: Vec<String>,
    questions: Vec<Question>,
    rules: Vec<Rule>,
}

struct Question {
    question: String,
    /// Oldest first.
    answers: Vec<String>,
    by_agent: bool,
}

struct Rule {
    text: String,
    scope: &'static str,
    /// Where a `(from ...)` marker says it belongs; empty for the node it stands in.
    from: Option<String>,
}

/// The parts of a `node.md`, as `scripts/build.py` `parse_node` reads it.
struct Parsed {
    status: String,
    same_as: String,
    handoff: String,
    title: String,
    sections: HashMap<String, String>,
}

fn parse_node(text: &str) -> Parsed {
    let text = text.replace("\r\n", "\n");
    let mut status = String::new();
    let mut same_as = String::new();
    let mut handoff = String::new();
    let mut rest = text.as_str();
    if let Some(after) = text.strip_prefix("---\n") {
        if let Some(end) = after.find("\n---\n") {
            for line in after[..end].lines() {
                if let Some((key, value)) = line.split_once(':') {
                    let value = value.trim().to_string();
                    match key.trim() {
                        "status" => status = value,
                        "same-as" => same_as = value,
                        "handoff" => handoff = value,
                        _ => {}
                    }
                }
            }
            rest = &after[end + 5..];
        }
    }
    let title = rest
        .lines()
        .find_map(|line| line.strip_prefix("# "))
        .map(|title| title.trim().to_string())
        .unwrap_or_default();
    let mut sections: HashMap<String, String> = HashMap::new();
    let mut current: Option<(String, Vec<&str>)> = None;
    for line in rest.lines() {
        if let Some(name) = line.strip_prefix("## ") {
            if let Some((name, lines)) = current.take() {
                sections.insert(name, lines.join("\n").trim().to_string());
            }
            current = Some((name.trim().to_string(), Vec::new()));
        } else if let Some((_, lines)) = current.as_mut() {
            lines.push(line);
        }
    }
    if let Some((name, lines)) = current {
        sections.insert(name, lines.join("\n").trim().to_string());
    }
    Parsed {
        status,
        same_as,
        handoff,
        title,
        sections,
    }
}

/// The folder a `Children` line names: `- [title](./folder/) — summary (status)`.
fn child_folder(line: &str) -> Option<String> {
    let after = line.trim().strip_prefix("- [")?;
    let target = after.split_once("](./")?.1;
    let folder = target.split_once(')')?.0.trim_end_matches('/');
    (!folder.is_empty() && !folder.contains('/')).then(|| folder.to_string())
}

fn slug_fits(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 80
        && slug.split('-').all(|word| {
            !word.is_empty()
                && word
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

fn shown(path: &str) -> &str {
    if path.is_empty() { "." } else { path }
}

fn join(path: &str, name: &str) -> String {
    if path.is_empty() {
        name.to_string()
    } else {
        format!("{path}/{name}")
    }
}

/// `link` resolved against the folder `path`; `None` when it leaves the root.
fn resolve(path: &str, link: &str) -> Option<String> {
    let mut parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    for part in link.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

/// Reads the folder `dir` and everything below it into `nodes`, parent first.
fn read_folder(
    dir: &Path,
    path: &str,
    nodes: &mut Vec<Node>,
    problems: &mut Vec<String>,
) -> Result<()> {
    let file = dir.join("node.md");
    let Ok(raw) = std::fs::read_to_string(&file) else {
        problems.push(format!("{}: the folder has no node.md", shown(path)));
        return Ok(());
    };
    let parsed = parse_node(&raw);
    let mut folders: Vec<String> = Vec::new();
    let entries = std::fs::read_dir(dir)
        .map_err(|error| CliError::new(format!("cannot read {}: {error}", dir.display())))?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || !entry.path().is_dir() {
            continue;
        }
        folders.push(name);
    }
    folders.sort();
    let listed: Vec<String> = parsed
        .sections
        .get("Children")
        .map(|text| text.lines().filter_map(child_folder).collect())
        .unwrap_or_default();
    let mut order: Vec<String> = Vec::new();
    for name in &listed {
        if folders.contains(name) && !order.contains(name) {
            order.push(name.clone());
        }
    }
    for name in &folders {
        if !order.contains(name) {
            order.push(name.clone());
        }
    }
    let name = path.rsplit('/').next().unwrap_or_default();
    let summary = parsed
        .sections
        .get("Summary")
        .and_then(|text| text.lines().next())
        .unwrap_or_default()
        .trim()
        .to_string();
    let section = |name: &str| parsed.sections.get(name).cloned().unwrap_or_default();
    let mut questions = Vec::new();
    let mut rules = Vec::new();
    let at = shown(path);
    let scope = if path.is_empty() {
        "feature"
    } else if !path.contains('/') {
        "story"
    } else {
        "node"
    };
    let lines_of = |name: &str, problems: &mut Vec<String>| -> Vec<String> {
        let mut out = Vec::new();
        for line in section(name).lines() {
            if line.trim().is_empty() {
                continue;
            }
            match line.trim().strip_prefix("- ") {
                Some(item) => out.push(item.trim().to_string()),
                None => problems.push(format!(
                    "{at}: a line in {name} does not start with `- `: {line}"
                )),
            }
        }
        out
    };
    for item in lines_of("Decisions", problems) {
        let (text, by_agent) = match item.strip_suffix("(default)") {
            Some(text) => (text.trim_end().to_string(), true),
            None => (item.clone(), false),
        };
        let mut parts = text.split(" → ").map(|part| part.trim().to_string());
        let question = parts.next().unwrap_or_default();
        let answers: Vec<String> = parts.collect();
        if answers.is_empty() {
            problems.push(format!("{at}: a Decisions line has no ` → `: - {item}"));
            continue;
        }
        questions.push(Question {
            question,
            answers,
            by_agent,
        });
    }
    for item in lines_of("Open questions", problems) {
        questions.push(Question {
            question: item,
            answers: Vec::new(),
            by_agent: false,
        });
    }
    for item in lines_of("Constraints", problems) {
        let mut text = item.clone();
        let mut from = None;
        if text.ends_with(')') {
            if let Some(open) = text.rfind(" (from ") {
                from = Some(text[open + 7..text.len() - 1].trim().to_string());
                text.truncate(open);
                text = text.trim_end().to_string();
            }
        }
        rules.push(Rule { text, scope, from });
    }
    nodes.push(Node {
        path: path.to_string(),
        slug: name.to_string(),
        title: parsed.title.clone(),
        summary,
        situation: section("Situation"),
        behaviour: section("Behavior"),
        status: parsed.status.clone(),
        same_as: parsed.same_as.clone(),
        handoff: parsed.handoff.clone(),
        children: order.iter().map(|name| join(path, name)).collect(),
        questions,
        rules,
    });
    for name in order {
        read_folder(&dir.join(&name), &join(path, &name), nodes, problems)?;
    }
    Ok(())
}

/// The nodes of the folder, parent first, siblings in order; or the problems.
fn read_tree(root: &Path) -> Result<std::result::Result<Vec<Node>, Vec<String>>> {
    if !root.is_dir() {
        bail!("{} is not a folder; {USAGE}", root.display());
    }
    let mut nodes = Vec::new();
    let mut problems = Vec::new();
    read_folder(root, "", &mut nodes, &mut problems)?;
    let paths: HashSet<&str> = nodes.iter().map(|node| node.path.as_str()).collect();
    for node in &nodes {
        let at = shown(&node.path);
        if !node.path.is_empty() && !slug_fits(&node.slug) {
            problems.push(format!(
                "{at}: the folder name {:?} is not a valid slug (lowercase letters, digits and -)",
                node.slug
            ));
        }
        if node.title.is_empty() {
            problems.push(format!("{at}: the node has no title (a `# ` line)"));
        }
        if !matches!(
            node.status.as_str(),
            "open" | "done" | "out-of-scope" | "stale"
        ) {
            problems.push(format!("{at}: unknown status {:?}", node.status));
        }
        if !node.same_as.is_empty() && !node.handoff.is_empty() {
            problems.push(format!("{at}: both same-as and handoff are set"));
        }
        for (field, link) in [("same-as", &node.same_as), ("handoff", &node.handoff)] {
            if link.is_empty() {
                continue;
            }
            match resolve(&node.path, link) {
                Some(target) if paths.contains(target.as_str()) => {}
                _ => problems.push(format!("{at}: {field} {link}: no such node folder")),
            }
            if !node.children.is_empty() {
                problems.push(format!("{at}: {field} on a node that has child folders"));
            }
        }
    }
    for node in &nodes {
        for rule in &node.rules {
            if let Some(from) = &rule.from {
                if !paths.contains(from.as_str()) || from.is_empty() {
                    problems.push(format!(
                        "{}: the `(from {from})` path names no node: - {}",
                        shown(&node.path),
                        rule.text
                    ));
                }
            }
        }
    }
    if problems.is_empty() {
        Ok(Ok(nodes))
    } else {
        Ok(Err(problems))
    }
}

fn counts(nodes: &[Node]) -> String {
    let stories = nodes
        .iter()
        .filter(|node| !node.path.is_empty() && !node.path.contains('/'))
        .count();
    let same_as = nodes.iter().filter(|node| !node.same_as.is_empty()).count();
    let handoffs = nodes.iter().filter(|node| !node.handoff.is_empty()).count();
    let questions: Vec<&Question> = nodes.iter().flat_map(|node| &node.questions).collect();
    let by_default = questions.iter().filter(|q| q.by_agent).count();
    let open = questions.iter().filter(|q| q.answers.is_empty()).count();
    let rules: usize = nodes.iter().map(|node| node.rules.len()).sum();
    format!(
        "{} nodes ({stories} stories), {same_as} same-as, {handoffs} handoffs, {} questions ({by_default} by default, {open} open), {rules} rules",
        nodes.len(),
        questions.len()
    )
}

fn refuse(problems: &[String], sent: bool) -> CliError {
    for problem in problems {
        eprintln!("{problem}");
    }
    let n = problems.len();
    CliError::new(format!(
        "invalid_import: {n} problem{}; {}",
        if n == 1 { "" } else { "s" },
        if sent {
            "nothing was imported"
        } else {
            "nothing was sent"
        }
    ))
}

/// The body of `POST .../behaviour/import`.
fn body_of(nodes: &[Node]) -> String {
    let key = |path: &str| {
        if path.is_empty() {
            ".".to_string()
        } else {
            path.to_string()
        }
    };
    // Each rule belongs to the node it stands in, or the one its `(from ...)` names.
    let mut held: HashMap<&str, Vec<&Rule>> = HashMap::new();
    for node in nodes {
        for rule in &node.rules {
            let at = rule.from.as_deref().unwrap_or(node.path.as_str());
            held.entry(at).or_default().push(rule);
        }
    }
    let items: Vec<String> = nodes
        .iter()
        .map(|node| {
            let parent = match node.path.rsplit_once('/') {
                Some((parent, _)) => quote(&key(parent)),
                None if node.path.is_empty() => "null".to_string(),
                None => quote("."),
            };
            let status = match node.status.as_str() {
                "out-of-scope" => "out_of_scope",
                "stale" => "open",
                other => other,
            };
            let mut fields = vec![
                format!("\"key\":{}", quote(&key(&node.path))),
                format!("\"parent\":{parent}"),
                format!("\"slug\":{}", quote(&node.slug)),
                format!("\"title\":{}", quote(&node.title)),
                format!("\"summary\":{}", quote(&node.summary)),
                format!("\"situation\":{}", quote(&node.situation)),
                format!("\"behaviour\":{}", quote(&node.behaviour)),
                format!("\"status\":{}", quote(status)),
            ];
            for (name, link) in [("same_as", &node.same_as), ("handoff", &node.handoff)] {
                if !link.is_empty() {
                    let target = resolve(&node.path, link).unwrap_or_default();
                    fields.push(format!("\"{name}\":{}", quote(&key(&target))));
                }
            }
            let questions: Vec<String> = node
                .questions
                .iter()
                .map(|q| {
                    let by = if q.by_agent { "agent" } else { "human" };
                    let answers: Vec<String> = q
                        .answers
                        .iter()
                        .map(|a| format!("{{\"answer\":{},\"by\":\"{by}\"}}", quote(a)))
                        .collect();
                    format!(
                        "{{\"question\":{},\"answers\":[{}]}}",
                        quote(&q.question),
                        answers.join(",")
                    )
                })
                .collect();
            fields.push(format!("\"questions\":[{}]", questions.join(",")));
            let rules: Vec<String> = held
                .get(node.path.as_str())
                .map(|rules| {
                    rules
                        .iter()
                        .map(|rule| {
                            format!(
                                "{{\"text\":{},\"scope\":\"{}\"}}",
                                quote(&rule.text),
                                rule.scope
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            fields.push(format!("\"rules\":[{}]", rules.join(",")));
            format!("{{{}}}", fields.join(","))
        })
        .collect();
    format!("{{\"nodes\":[{}]}}", items.join(","))
}

/// `omni behaviour import <folder> [--check]`
pub fn import(args: &Args, folder: Option<&str>) -> Result<()> {
    let Some(folder) = folder else {
        bail!("this command needs a folder: {USAGE}");
    };
    let nodes = match read_tree(Path::new(folder))? {
        Ok(nodes) => nodes,
        Err(problems) => return Err(refuse(&problems, false)),
    };
    if args.check {
        say(&format!("would import {}", counts(&nodes)));
        return Ok(());
    }
    let agent = agent(args)?;
    let base = behaviour_path(args, &agent)?;
    let body = body_of(&nodes);
    let response = agent.send_unchecked("POST", &format!("{base}/import"), body.as_bytes())?;
    if response.status == 422 {
        if let Ok(answer) = Value::parse(&response.text()) {
            if answer.str_at("error") == Some("invalid_import") {
                let problems: Vec<String> = answer
                    .get("problems")
                    .and_then(Value::as_array)
                    .unwrap_or_default()
                    .iter()
                    .map(|problem| {
                        format!(
                            "{}: {}",
                            problem.str_at("key").unwrap_or("."),
                            problem.str_at("problem").unwrap_or("")
                        )
                    })
                    .collect();
                return Err(refuse(&problems, true));
            }
        }
    }
    crate::commands::checked(response)?;
    say(&format!("imported {}", counts(&nodes)));
    Ok(())
}
