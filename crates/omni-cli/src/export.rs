//! `omni export --project <p> [--feature <name>] --out <dir>`: write the plan
//! of one scope to a folder a builder can work from without the API. The
//! folder gets `manifest.json`, one `.html` file per HTML doc under its
//! pillar, one tag-format file per board of each design under `ui/<doc>/`,
//! and `tokens.json` with `tokens.css` from the app-level design system.

use crate::args::Args;
use crate::commands::{Agent, agent, say};
use crate::components::is_node_id;
use crate::error::{CliError, Result, bail};
use crate::json::Value;
use crate::projects::{creation_feature, project_id};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A file to write and the line of the manifest that lists it.
struct Exported {
    path: String,
    content: String,
    doc_id: String,
    pillar: String,
    title: String,
    seq: String,
    board: Option<(String, String)>,
}

fn parsed(text: &str, what: &str) -> Result<Value> {
    Value::parse(text).map_err(|_| {
        CliError::new(format!(
            "the server's answer about {what} is not readable; is it an OmniState API?"
        ))
    })
}

fn get(agent: &Agent, path: &str, what: &str) -> Result<Value> {
    parsed(&agent.request("GET", path, None)?.text(), what)
}

/// A name for a file: the title's letters and digits, joined by `-`.
fn slug(title: &str) -> String {
    let words: Vec<String> = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    if words.is_empty() {
        "doc".to_string()
    } else {
        words.join("-")
    }
}

/// The folder name of a doc: its title and the end of its ID, which tells two
/// docs with one title apart.
fn doc_stem(title: &str, id: &str) -> String {
    let tail = &id[id.len().saturating_sub(8)..];
    format!("{}-{tail}", slug(title))
}

/// `omni export --project <p> [--feature <name>] --out <dir>`
pub fn run(args: &Args) -> Result<()> {
    const USAGE: &str = "omni export --project <name|id> [--feature <name|id>] --out <dir>";
    let Some(out) = args.out.as_deref() else {
        bail!("an export needs the folder to write to: {USAGE}");
    };
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let feature = creation_feature(args, &agent, &project)?;
    let base = format!("/api/v1/w/{}/projects/{project}", agent.workspace_id);

    let docs_of = |scope: &str| -> Result<Vec<Value>> {
        let answer = get(&agent, &format!("{base}/docs?feature={scope}"), "the docs")?;
        Ok(answer
            .get("docs")
            .and_then(Value::as_array)
            .unwrap_or_default()
            .to_vec())
    };
    let app_docs = docs_of("global")?;
    let docs = match &feature {
        Some(feature) => docs_of(feature)?,
        None => app_docs.clone(),
    };

    // Every read comes before the first write, so a failure leaves no half.
    let mut files: Vec<Exported> = Vec::new();
    for doc in &docs {
        let field = |name: &str| doc.str_at(name).unwrap_or_default().to_string();
        let (id, kind, pillar, title) =
            (field("id"), field("kind"), field("pillar"), field("title"));
        if !crate::projects::is_uuid(&id)
            || !matches!(
                pillar.as_str(),
                "spec" | "ui" | "architecture" | "db" | "research"
            )
        {
            bail!("the server listed a doc {id:?} that is not a doc; is it an OmniState API?");
        }
        let doc_path = format!("{base}/docs/{id}");
        let seq = latest_seq(&agent, &doc_path)?;
        let stem = doc_stem(&title, &id);
        match kind.as_str() {
            "html" => {
                let head = get(&agent, &format!("{doc_path}/html"), "the doc")?;
                files.push(Exported {
                    path: format!("{pillar}/{stem}.html"),
                    content: head.str_at("html").unwrap_or_default().to_string(),
                    doc_id: id,
                    pillar,
                    title,
                    seq,
                    board: None,
                });
            }
            "design" => {
                let head = get(
                    &agent,
                    &format!("{doc_path}/html?view=outline"),
                    "the design",
                )?;
                let mut boards = Vec::new();
                for page in head
                    .get("outline")
                    .and_then(Value::as_array)
                    .unwrap_or_default()
                {
                    for board in page
                        .get("children")
                        .and_then(Value::as_array)
                        .unwrap_or_default()
                    {
                        boards.push((
                            board.str_at("id").unwrap_or_default().to_string(),
                            board.str_at("name").unwrap_or_default().to_string(),
                        ));
                    }
                }
                for (board, name) in boards {
                    if !is_node_id(&board) {
                        bail!("the server named a board {board:?}; is it an OmniState API?");
                    }
                    let one = get(&agent, &format!("{doc_path}/boards/{board}"), "the board")?;
                    files.push(Exported {
                        path: format!("{pillar}/{stem}/{}-{board}.tags", slug(&name)),
                        content: one.str_at("tags").unwrap_or_default().to_string(),
                        doc_id: id.clone(),
                        pillar: pillar.clone(),
                        title: title.clone(),
                        seq: seq.clone(),
                        board: Some((board, name)),
                    });
                }
            }
            _ => {}
        }
    }

    if let Some(system) = app_docs
        .iter()
        .find(|doc| doc.str_at("kind") == Some("design-system"))
    {
        let answer = get(&agent, &format!("{base}/tokens"), "the tokens")?;
        if let Some(tokens) = answer
            .get("tokens")
            .filter(|tokens| **tokens != Value::Null)
        {
            let modes: Vec<&str> = answer
                .get("modes")
                .and_then(Value::as_array)
                .unwrap_or_default()
                .iter()
                .filter_map(Value::as_str)
                .collect();
            let css = tokens_css(tokens, &modes)?;
            let id = system.str_at("id").unwrap_or_default();
            let seq = latest_seq(&agent, &format!("{base}/docs/{id}"))?;
            for (path, content) in [
                ("tokens.json", format!("{}\n", tokens.pretty())),
                ("tokens.css", css),
            ] {
                files.push(Exported {
                    path: path.to_string(),
                    content,
                    doc_id: id.to_string(),
                    pillar: "ui".to_string(),
                    title: system.str_at("title").unwrap_or_default().to_string(),
                    seq: seq.clone(),
                    board: None,
                });
            }
        }
    }

    let text = |s: &str| Value::String(s.to_string());
    let entries = files
        .iter()
        .map(|file| {
            let mut fields = vec![
                ("path".to_string(), text(&file.path)),
                ("doc_id".to_string(), text(&file.doc_id)),
                ("pillar".to_string(), text(&file.pillar)),
                ("title".to_string(), text(&file.title)),
                ("seq".to_string(), Value::Number(file.seq.clone())),
            ];
            if let Some((id, name)) = &file.board {
                fields.push(("board_id".to_string(), text(id)));
                fields.push(("board".to_string(), text(name)));
            }
            Value::Object(fields)
        })
        .collect();
    let manifest = Value::Object(vec![("files".to_string(), Value::Array(entries))]);

    let dir = Path::new(out);
    for file in &files {
        write(&dir.join(&file.path), &file.content)?;
    }
    write(
        &dir.join("manifest.json"),
        &format!("{}\n", manifest.pretty()),
    )?;
    if args.json {
        say(&manifest.to_string());
    } else {
        say(&format!("wrote {} files to {out}", files.len() + 1));
    }
    Ok(())
}

/// The number of the doc's latest write in `doc_changes`.
fn latest_seq(agent: &Agent, doc_path: &str) -> Result<String> {
    let answer = get(
        agent,
        &format!("{doc_path}/activity?limit=1"),
        "the activity",
    )?;
    match answer
        .get("entries")
        .and_then(Value::as_array)
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.get("seq"))
    {
        Some(Value::Number(seq)) => Ok(seq.clone()),
        _ => bail!("the server's activity of the doc has no write; is it an OmniState API?"),
    }
}

fn write(path: &PathBuf, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            CliError::new(format!(
                "cannot make the directory {}: {e}",
                parent.display()
            ))
        })?;
    }
    std::fs::write(path, content)
        .map_err(|e| CliError::new(format!("cannot write {}: {e}", path.display())))
}

// tokens.css ----------------------------------------------------------------

/// One value of a token in a mode as the variables it becomes: `(variable,
/// css value)`, in order.
type Variables = Vec<(String, String)>;

/// The tokens of the tree, each with its path and its raw value in a mode.
fn collect<'a>(node: &'a Value, path: &mut Vec<&'a str>, out: &mut Vec<(Vec<&'a str>, &'a Value)>) {
    let Value::Object(fields) = node else { return };
    for (key, value) in fields {
        if key.starts_with('$') {
            continue;
        }
        path.push(key);
        if value.get("$value").is_some() {
            out.push((path.clone(), value));
        } else {
            collect(value, path, out);
        }
        path.pop();
    }
}

fn mode_value<'a>(token: &'a Value, mode: &str) -> &'a Value {
    token
        .get("$extensions")
        .and_then(|e| e.get("omnistate"))
        .and_then(|o| o.get("modes"))
        .and_then(|m| m.get(mode))
        .or_else(|| token.get("$value"))
        .expect("a token has a $value")
}

fn variable(path: &[&str]) -> String {
    format!("--{}", path.join("-"))
}

/// `{color.paper}` as `var(--color-paper)`; `None` for anything else.
fn reference(value: &Value) -> Option<String> {
    let text = value.as_str()?;
    let inner = text.strip_prefix('{')?.strip_suffix('}')?;
    Some(format!("var(--{})", inner.replace('.', "-")))
}

/// A color, length, number or reference as CSS. A length is in `px`.
fn scalar(value: &Value, length: bool) -> String {
    if let Some(var) = reference(value) {
        return var;
    }
    match value {
        Value::String(text) => text.clone(),
        Value::Number(n) if length => format!("{n}px"),
        other => other.to_string(),
    }
}

fn family(value: &Value) -> String {
    match reference(value) {
        Some(var) => var,
        None => {
            let text = value.as_str().unwrap_or_default();
            format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
        }
    }
}

const TYPOGRAPHY_FIELDS: [(&str, &str, bool); 5] = [
    ("fontFamily", "font-family", false),
    ("fontSize", "font-size", true),
    ("fontWeight", "font-weight", false),
    ("lineHeight", "line-height", false),
    ("letterSpacing", "letter-spacing", true),
];

/// The variables of one token in one mode.
fn variables_of(
    tokens: &BTreeMap<String, &Value>,
    mode: &str,
    path: &[&str],
    token: &Value,
) -> Variables {
    let name = variable(path);
    let value = mode_value(token, mode);
    match path[0] {
        "typography" => {
            // A reference to a whole typography token stands for its fields,
            // which are the ones of the token at the end of the chain.
            let mut fields = value;
            for _ in 0..64 {
                let Some(target) = value_ref_path(fields).and_then(|p| tokens.get(&p)) else {
                    break;
                };
                fields = mode_value(target, mode);
            }
            let whole = value_ref_path(value);
            TYPOGRAPHY_FIELDS
                .iter()
                .filter_map(|(key, suffix, length)| {
                    let field = fields.get(key)?;
                    let css = match &whole {
                        Some(path) => format!("var(--{}-{suffix})", path.replace('.', "-")),
                        None if *key == "fontFamily" => family(field),
                        None => scalar(field, *length),
                    };
                    Some((format!("{name}-{suffix}"), css))
                })
                .collect()
        }
        "shadow" => {
            if let Some(var) = reference(value) {
                return vec![(name, var)];
            }
            let length = |key: &str| {
                value
                    .get(key)
                    .map_or("0px".to_string(), |v| scalar(v, true))
            };
            let color = value
                .get("color")
                .map_or(String::new(), |v| scalar(v, false));
            vec![(
                name,
                format!(
                    "{} {} {} {} {color}",
                    length("offsetX"),
                    length("offsetY"),
                    length("blur"),
                    length("spread")
                ),
            )]
        }
        group => {
            let css = scalar(value, group == "spacing" || group == "radius");
            vec![(name, css)]
        }
    }
}

/// The dotted path inside a whole-value reference, `{typography.body}`.
fn value_ref_path(value: &Value) -> Option<String> {
    let text = value.as_str()?;
    Some(text.strip_prefix('{')?.strip_suffix('}')?.to_string())
}

/// `tokens.css`: the first mode in `:root`, each other mode in
/// `[data-mode="<mode>"]` with only the variables it changes.
pub(crate) fn tokens_css(tree: &Value, modes: &[&str]) -> Result<String> {
    let mut found = Vec::new();
    for group in ["color", "spacing", "radius", "typography", "shadow"] {
        if let Some(node) = tree.get(group) {
            let mut path = vec![group];
            collect(node, &mut path, &mut found);
        }
    }
    let by_path: BTreeMap<String, &Value> = found
        .iter()
        .map(|(path, token)| (path.join("."), *token))
        .collect();

    // Two tokens must not share a variable.
    let mut owners: BTreeMap<String, String> = BTreeMap::new();
    for (path, token) in &found {
        let dotted = path.join(".");
        let mode = modes.first().copied().unwrap_or_default();
        for (name, _) in variables_of(&by_path, mode, path, token) {
            if let Some(other) = owners.insert(name.clone(), dotted.clone())
                && other != dotted
            {
                bail!(
                    "the tokens {other} and {dotted} both become the variable {name}; rename one"
                );
            }
        }
    }

    let mut blocks = Vec::new();
    let mut first: BTreeMap<String, String> = BTreeMap::new();
    for (index, mode) in modes.iter().enumerate() {
        let mut lines = Vec::new();
        for (path, token) in &found {
            for (name, css) in variables_of(&by_path, mode, path, token) {
                if index == 0 {
                    first.insert(name.clone(), css.clone());
                } else if first.get(&name) == Some(&css) {
                    continue;
                }
                lines.push(format!("  {name}: {css};\n"));
            }
        }
        let selector = if index == 0 {
            ":root".to_string()
        } else {
            format!("[data-mode=\"{}\"]", mode.replace('"', "\\\""))
        };
        blocks.push(format!("{selector} {{\n{}}}\n", lines.concat()));
    }
    Ok(blocks.join("\n"))
}
