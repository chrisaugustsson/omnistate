//! `omni version save`, `omni version diff` and `omni doc remove`: save a change
//! set's draft of a doc as an unchangeable version, compare two versions, and
//! remove a doc by saving a version that marks it removed. The project is found
//! as for `omni changeset`.

use crate::args::Args;
use crate::changesets::{is_uuid, project_id};
use crate::commands::{agent, say};
use crate::error::{CliError, Result, bail};
use crate::json::Value;

fn parsed(text: &str, what: &str) -> Result<Value> {
    Value::parse(text).map_err(|_| {
        CliError::new(format!(
            "the server's answer about {what} is not readable; is it an OmniState API?"
        ))
    })
}

fn doc_id(id: Option<&str>, usage: &str) -> Result<String> {
    match id {
        None => bail!("this command needs the doc's ID: {usage}"),
        Some(id) if is_uuid(id) => Ok(id.to_ascii_lowercase()),
        Some(id) => bail!("{id:?} is not a doc ID (a UUID); `omni doc show <doc>` takes the ID"),
    }
}

fn version_id(id: Option<&str>, usage: &str) -> Result<String> {
    match id {
        None => bail!("this command needs two version IDs: {usage}"),
        Some(id) if is_uuid(id) => Ok(id.to_ascii_lowercase()),
        Some(id) => bail!("{id:?} is not a version ID (a UUID); `omni version save` prints it"),
    }
}

fn changeset_option(args: &Args, usage: &str) -> Result<String> {
    match args.changeset.as_deref() {
        None => bail!("this command needs --changeset <id>: {usage}"),
        Some(id) if is_uuid(id) => Ok(id.to_ascii_lowercase()),
        Some(id) => {
            bail!("--changeset {id:?} is not a change set ID; `omni changeset list` shows them")
        }
    }
}

/// `omni version save <doc> --changeset <id>`
pub fn save(args: &Args, doc: Option<&str>) -> Result<()> {
    const USAGE: &str = "omni version save <doc> --changeset <id>";
    let doc = doc_id(doc, USAGE)?;
    let changeset = changeset_option(args, USAGE)?;
    let not_found = format!(
        "not_found: change set {changeset} has no draft of doc {doc}; `omni doc open <doc> --changeset <id>` makes one"
    );
    send(args, &changeset, &doc, "versions", &not_found, |version| {
        let nodes = version
            .get("nodes")
            .and_then(Value::as_array)
            .map_or(0, <[Value]>::len);
        format!(
            "saved version {}  of doc {doc}  ({nodes} nodes)",
            version.str_at("id").unwrap_or("?")
        )
    })
}

/// `omni doc remove <doc> --changeset <id>`
pub fn remove(args: &Args, doc: Option<&str>) -> Result<()> {
    const USAGE: &str = "omni doc remove <doc> --changeset <id>";
    let doc = doc_id(doc, USAGE)?;
    let changeset = changeset_option(args, USAGE)?;
    let not_found = format!(
        "not_found: change set {changeset} has no draft of doc {doc}; `omni doc open <doc> --changeset <id>` makes one"
    );
    send(args, &changeset, &doc, "remove", &not_found, |version| {
        format!(
            "removed doc {doc}  in version {}  (the history stays)",
            version.str_at("id").unwrap_or("?")
        )
    })
}

fn send(
    args: &Args,
    changeset: &str,
    doc: &str,
    route: &str,
    not_found: &str,
    line: impl Fn(&Value) -> String,
) -> Result<()> {
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let response = agent.request_or_not_found(
        "POST",
        &format!(
            "/api/v1/w/{}/projects/{project}/changesets/{changeset}/docs/{doc}/{route}",
            agent.workspace_id
        ),
        Some(b"{}"),
        not_found,
    )?;
    let text = response.text();
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    let answer = parsed(&text, "the version")?;
    say(&line(answer.get("version").unwrap_or(&answer)));
    Ok(())
}

/// `omni version diff <a> <b>`
pub fn diff(args: &Args, from: Option<&str>, to: Option<&str>) -> Result<()> {
    const USAGE: &str = "omni version diff <a> <b>";
    let from = version_id(from, USAGE)?;
    let to = version_id(to, USAGE)?;
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let not_found = format!(
        "not_found: there is no version {from} or {to} in this project; `omni version save` prints a version's ID"
    );
    let response = agent.request_or_not_found(
        "GET",
        &format!(
            "/api/v1/w/{}/projects/{project}/versions/{from}/diff/{to}",
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
    let answer = parsed(&text, "the comparison")?;
    let diff = answer.get("diff").unwrap_or(&answer);
    let list = |name: &str| diff.get(name).and_then(Value::as_array).unwrap_or_default();
    let mut lines = 0;
    for word in ["added", "removed", "changed", "moved"] {
        for node in list(word) {
            let mut line = format!(
                "{word}  {}  {}",
                node.str_at("id").unwrap_or("?"),
                node.str_at("type").unwrap_or("?")
            );
            if word == "moved" {
                line.push_str(&format!(
                    "  to {}",
                    node.str_at("to_parent").unwrap_or("the top level")
                ));
                match node.get("to_after") {
                    Some(Value::String(after)) => line.push_str(&format!(" after {after}")),
                    Some(Value::Null) => line.push_str(" first"),
                    _ => {}
                }
            }
            say(&line);
            lines += 1;
        }
    }
    if lines == 0 {
        say("no differences");
    }
    Ok(())
}
