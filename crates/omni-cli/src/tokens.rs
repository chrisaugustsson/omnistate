//! `omni tokens import` and `omni tokens export`: replace the design tokens of a
//! change set's draft with a DTCG file, and write the tokens of a draft or a
//! version as DTCG JSON, whole or one file per mode (docs/formats/tokens.md).
//! The project is found as for `omni changeset`.

use crate::args::Args;
use crate::changesets::{is_uuid, project_id};
use crate::commands::{Agent, agent, say};
use crate::error::{CliError, Result, bail};
use crate::json::Value;
use std::path::Path;

fn parsed(text: &str, what: &str) -> Result<Value> {
    Value::parse(text).map_err(|_| {
        CliError::new(format!(
            "the server's answer about {what} is not readable; is it an OmniState API?"
        ))
    })
}

fn modes_of(answer: &Value) -> Vec<&str> {
    answer
        .get("modes")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
        .collect()
}

/// `omni tokens import <file> --changeset <id>`
pub fn import(args: &Args, file: Option<&str>) -> Result<()> {
    const USAGE: &str = "omni tokens import <file> --changeset <id>";
    let changeset = match args.changeset.as_deref() {
        None => bail!("this command needs --changeset <id>: {USAGE}"),
        Some(id) if is_uuid(id) => id.to_ascii_lowercase(),
        Some(id) => {
            bail!("--changeset {id:?} is not a change set ID; `omni changeset list` shows them")
        }
    };
    let Some(file) = file else {
        bail!("this command needs the token file to import: {USAGE}");
    };
    let text = std::fs::read_to_string(file)
        .map_err(|e| CliError::new(format!("cannot read the token file {file}: {e}")))?;
    if let Err(reason) = Value::parse(&text) {
        bail!(
            "the token file {file} is not JSON ({reason}); write the DTCG format of docs/formats/tokens.md"
        );
    }

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let body = format!("{{\"tokens\":{}}}", text.trim());
    let response = agent.request(
        "PUT",
        &format!(
            "{}/changesets/{changeset}/tokens",
            project_path(&agent, &project)
        ),
        Some(body.as_bytes()),
    )?;
    let text = response.text();
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    let answer = parsed(&text, "the tokens")?;
    say(&format!(
        "imported the token tree into draft {} of the design system, in change set {changeset}",
        answer.str_at("draft_id").unwrap_or("?")
    ));
    say(&format!("modes: {}", modes_of(&answer).join(", ")));
    Ok(())
}

fn project_path(agent: &Agent, project: &str) -> String {
    format!("/api/v1/w/{}/projects/{project}", agent.workspace_id)
}

/// A mode becomes part of a file name, and the server named it.
fn file_name_of(mode: &str) -> Result<String> {
    let safe = !mode.is_empty()
        && mode
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_');
    if !safe {
        bail!(
            "the server named a mode {mode:?}, which is not a mode name; is it an OmniState API?"
        );
    }
    Ok(format!("{mode}.tokens.json"))
}

fn write(path: &Path, tokens: &Value) -> Result<()> {
    std::fs::write(path, format!("{}\n", tokens.pretty()))
        .map_err(|e| CliError::new(format!("cannot write {}: {e}", path.display())))
}

/// `omni tokens export [--changeset <id> | --version <v>] [--out <file>] [--per-mode --out <dir>]`
pub fn export(args: &Args) -> Result<()> {
    const USAGE: &str = "omni tokens export [--changeset <id> | --version <v>] [--out <file>] [--per-mode --out <dir>]";
    if args.changeset.is_some() && args.doc_version.is_some() {
        bail!(
            "give --changeset or --version, not both: --changeset reads a draft, --version reads a saved version"
        );
    }
    if args.per_mode && args.out.is_none() {
        bail!("--per-mode writes one file per mode, so it needs --out <directory>: {USAGE}");
    }
    let changeset = match args.changeset.as_deref() {
        Some(id) if is_uuid(id) => Some(id.to_ascii_lowercase()),
        Some(id) => {
            bail!("--changeset {id:?} is not a change set ID; `omni changeset list` shows them")
        }
        None => None,
    };
    let version = match args.doc_version.as_deref() {
        Some(version) if is_uuid(version) => Some(version.to_ascii_lowercase()),
        Some(version) => bail!("--version {version:?} is not a version ID (a UUID)"),
        None => None,
    };

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let base = project_path(&agent, &project);
    let (path, mut query) = match (&changeset, &version) {
        (Some(changeset), _) => (format!("{base}/changesets/{changeset}/tokens"), Vec::new()),
        (None, Some(version)) => (format!("{base}/tokens"), vec![format!("version={version}")]),
        (None, None) => (format!("{base}/tokens"), Vec::new()),
    };
    let read = |query: &[String]| -> Result<crate::http::Response> {
        let full = match query {
            [] => path.clone(),
            query => format!("{path}?{}", query.join("&")),
        };
        agent.request("GET", &full, None)
    };

    let response = read(&query)?;
    let text = response.text();
    if args.json && !args.per_mode && args.out.is_none() {
        say(text.trim_end());
        return Ok(());
    }
    let answer = parsed(&text, "the tokens")?;
    let Some(tokens) = answer
        .get("tokens")
        .filter(|tokens| **tokens != Value::Null)
    else {
        match changeset {
            Some(changeset) => bail!(
                "change set {changeset} has no tokens in its draft yet; `omni tokens import <file> --changeset <id>` adds them"
            ),
            None => bail!(
                "the design system has no mainline version yet, so there are no tokens to export; read a draft with --changeset <id>"
            ),
        }
    };

    if !args.per_mode {
        match args.out.as_deref() {
            None => say(&tokens.pretty()),
            Some(out) => {
                write(Path::new(out), tokens)?;
                say(&format!("wrote {out}"));
            }
        }
        return Ok(());
    }

    let dir = Path::new(args.out.as_deref().unwrap_or_default());
    // Every read comes before the first write, so a failure leaves no half.
    let mut files = Vec::new();
    for mode in modes_of(&answer) {
        let name = file_name_of(mode)?;
        query.push(format!("mode={mode}"));
        let response = read(&query)?;
        query.pop();
        let one = parsed(&response.text(), "the tokens of a mode")?;
        let Some(tree) = one.get("tokens").cloned() else {
            bail!("the server's answer has no tokens for the mode {mode}; is it an OmniState API?");
        };
        files.push((name, tree));
    }
    std::fs::create_dir_all(dir)
        .map_err(|e| CliError::new(format!("cannot make the directory {}: {e}", dir.display())))?;
    for (name, tree) in &files {
        let path = dir.join(name);
        write(&path, tree)?;
        say(&format!("wrote {}", path.display()));
    }
    Ok(())
}
