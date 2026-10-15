//! `omni doc`: start a doc in a change set, open it there, read it (mainline, a
//! version or a change set's draft), show its metadata and edit a draft with a
//! batch of operations. The project is found as for `omni changeset`.

use crate::args::Args;
use crate::changesets::{is_uuid, project_id};
use crate::commands::{Agent, agent, read_body, say};
use crate::error::{CliError, Result, bail};
use crate::json::{Value, quote};
use std::io::IsTerminal;

const KINDS: &str = "html, design or design-system";
const MODULES: &str = "prd, architecture, db or ui";

/// The text of an API response, for `--json`.
fn print_json(text: &str) {
    say(text.trim_end());
}

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

fn changeset_option(args: &Args, usage: &str) -> Result<String> {
    match args.changeset.as_deref() {
        None => bail!("this command needs --changeset <id>: {usage}"),
        Some(id) if is_uuid(id) => Ok(id.to_ascii_lowercase()),
        Some(id) => {
            bail!("--changeset {id:?} is not a change set ID; `omni changeset list` shows them")
        }
    }
}

fn project_path(agent: &Agent, project: &str) -> String {
    format!("/api/v1/w/{}/projects/{project}", agent.workspace_id)
}

/// `omni doc create --changeset <id> --kind <kind> --module <module> --title <text>`
pub fn create(args: &Args) -> Result<()> {
    const USAGE: &str = "omni doc create --changeset <id> --kind html|design|design-system --module prd|architecture|db|ui --title <text>";
    let changeset = changeset_option(args, USAGE)?;
    let kind = match args.kind.as_deref() {
        None => bail!("a doc needs --kind, one of {KINDS}: {USAGE}"),
        Some(kind @ ("html" | "design" | "design-system")) => kind,
        Some(kind) => bail!("--kind {kind:?} is not a kind of doc; use {KINDS}"),
    };
    let module = match args.module.as_deref() {
        None => bail!("a doc needs --module, one of {MODULES}: {USAGE}"),
        Some(module @ ("prd" | "architecture" | "db" | "ui")) => module,
        Some(module) => bail!("--module {module:?} is not a module; use {MODULES}"),
    };
    let fits = match kind {
        "html" => module != "ui",
        _ => module == "ui",
    };
    if !fits {
        bail!(
            "a {kind} doc does not go in the {module} module: html docs are prd, architecture or db, design and design-system docs are ui"
        );
    }
    let title = args.title.as_deref().map(str::trim).unwrap_or_default();
    if title.is_empty() {
        bail!("a doc needs --title <text>: {USAGE}");
    }

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let body = format!(
        "{{\"kind\":{},\"module\":{},\"title\":{}}}",
        quote(kind),
        quote(module),
        quote(title)
    );
    let response = agent.request(
        "POST",
        &format!(
            "{}/changesets/{changeset}/docs",
            project_path(&agent, &project)
        ),
        Some(body.as_bytes()),
    )?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let answer = parsed(&text, "the new doc")?;
    let doc = answer.get("doc").unwrap_or(&answer);
    let field = |value: &Value, name: &str| value.str_at(name).unwrap_or("?").to_string();
    say(&format!(
        "created doc {}  {}  {}",
        field(doc, "id"),
        field(doc, "kind"),
        field(doc, "title")
    ));
    if let Some(draft) = answer.get("draft") {
        say(&format!(
            "draft {} in change set {}",
            field(draft, "id"),
            field(draft, "changeset_id")
        ));
    }
    Ok(())
}

/// `omni doc open <doc> --changeset <id>`
pub fn open(args: &Args, doc: Option<&str>) -> Result<()> {
    const USAGE: &str = "omni doc open <doc> --changeset <id>";
    let doc = doc_id(doc, USAGE)?;
    let changeset = changeset_option(args, USAGE)?;
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let response = agent.request(
        "POST",
        &format!(
            "{}/changesets/{changeset}/docs/{doc}/open",
            project_path(&agent, &project)
        ),
        Some(b"{}"),
    )?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let answer = parsed(&text, "the draft")?;
    let draft = answer.get("draft").unwrap_or(&answer);
    let base = match draft.str_at("base_version_id") {
        Some(version) => format!("based on version {version}"),
        None => "based on no version".to_string(),
    };
    say(&format!(
        "draft {} of doc {doc} in change set {changeset}, {base}",
        draft.str_at("id").unwrap_or("?")
    ));
    Ok(())
}

/// `omni doc read <doc> [--changeset <id> | --version <v>]`
pub fn read(args: &Args, doc: Option<&str>) -> Result<()> {
    const USAGE: &str = "omni doc read <doc> [--changeset <id> | --version <v>]";
    let doc = doc_id(doc, USAGE)?;
    if args.changeset.is_some() && args.doc_version.is_some() {
        bail!(
            "give --changeset or --version, not both: --changeset reads a draft, --version reads a saved version"
        );
    }
    let draft_of = match args.changeset.is_some() {
        true => Some(changeset_option(args, USAGE)?),
        false => None,
    };
    let version = match args.doc_version.as_deref() {
        Some(version) if is_uuid(version) => Some(version.to_ascii_lowercase()),
        Some(version) => bail!("--version {version:?} is not a version ID (a UUID)"),
        None => None,
    };

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let base = project_path(&agent, &project);
    let path = match (&draft_of, &version) {
        (Some(changeset), _) => format!("{base}/changesets/{changeset}/docs/{doc}"),
        (None, Some(version)) => format!("{base}/docs/{doc}/html?version={version}"),
        (None, None) => format!("{base}/docs/{doc}/html"),
    };
    let response = agent.request("GET", &path, None)?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let answer = parsed(&text, "the doc")?;
    let (holder, what) = match draft_of {
        Some(_) => (answer.get("draft").unwrap_or(&answer), "the draft"),
        None => (&answer, "the version"),
    };
    if draft_of.is_none() && holder.get("version_id") == Some(&Value::Null) {
        bail!(
            "doc {doc} has no mainline version yet: it exists only in the change set that started it; read it with --changeset <id>"
        );
    }
    let Some(html) = holder.str_at("html") else {
        bail!("the server's answer has no HTML for {what}; is it an OmniState API?");
    };
    // The HTML as the API gave it, and nothing else.
    say(html);
    Ok(())
}

/// `omni doc show <doc>`
pub fn show(args: &Args, doc: Option<&str>) -> Result<()> {
    let doc = doc_id(doc, "omni doc show <doc>")?;
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let response = agent.request(
        "GET",
        &format!("{}/docs/{doc}", project_path(&agent, &project)),
        None,
    )?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let answer = parsed(&text, "the doc")?;
    let doc = answer.get("doc").unwrap_or(&answer);
    let field = |value: &Value, name: &str| value.str_at(name).unwrap_or("?").to_string();
    say(&format!(
        "doc {}  {}",
        field(doc, "id"),
        field(doc, "title")
    ));
    say(&format!(
        "kind: {}  module: {}",
        field(doc, "kind"),
        field(doc, "module")
    ));
    say(&format!(
        "mainline version: {}",
        doc.str_at("mainline_version_id").unwrap_or("none")
    ));
    let drafts = doc
        .get("drafts")
        .and_then(Value::as_array)
        .unwrap_or_default();
    if drafts.is_empty() {
        say("drafts: none");
    } else {
        say("drafts:");
    }
    for draft in drafts {
        let base = match draft.str_at("base_version_id") {
            Some(version) => format!("  based on version {version}"),
            None => String::new(),
        };
        say(&format!(
            "  {}  change set {}{base}",
            field(draft, "id"),
            field(draft, "changeset_id")
        ));
    }
    let claims = doc
        .get("claims")
        .and_then(Value::as_array)
        .unwrap_or_default();
    if claims.is_empty() {
        say("claims: none");
    } else {
        say("claims:");
    }
    for claim in claims {
        let agent = claim.get("agent");
        let part = |name: &str| agent.and_then(|a| a.str_at(name)).unwrap_or("?");
        say(&format!(
            "  {}  {}/{}  until {}",
            field(claim, "node"),
            part("run"),
            part("role"),
            field(claim, "expires_at")
        ));
    }
    Ok(())
}

/// `omni doc edit <doc> --changeset <id> [--body-file <path|->]`
pub fn edit(args: &Args, doc: Option<&str>) -> Result<()> {
    const USAGE: &str = "omni doc edit <doc> --changeset <id> [--body-file <path|->]";
    let doc = doc_id(doc, USAGE)?;
    let changeset = changeset_option(args, USAGE)?;
    let source = match args.body_file.as_deref() {
        Some(source) => source,
        None if std::io::stdin().is_terminal() => bail!(
            "no operations to send: pass --body-file <path> or pipe the batch (a JSON array) to standard input"
        ),
        None => "-",
    };
    let bytes = read_body(source)?;
    let batch = String::from_utf8(bytes)
        .map_err(|_| CliError::new("the operations are not valid UTF-8 text"))?;
    match Value::parse(&batch) {
        Ok(Value::Array(_)) => {}
        Ok(_) => bail!(
            "the operations must be a JSON array of operations, such as [{{\"op\":\"delete\",\"node\":\"k3f9x2qa\",\"rev\":1}}]"
        ),
        Err(why) => bail!("the operations are not valid JSON ({why}); send a JSON array"),
    }

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let body = format!("{{\"ops\":{}}}", batch.trim());
    let response = agent.request(
        "POST",
        &format!(
            "{}/changesets/{changeset}/docs/{doc}/ops",
            project_path(&agent, &project)
        ),
        Some(body.as_bytes()),
    )?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let answer = parsed(&text, "the edit")?;
    for result in answer
        .get("results")
        .and_then(Value::as_array)
        .unwrap_or_default()
    {
        let ids = |name: &str| -> Vec<&str> {
            result
                .get(name)
                .and_then(Value::as_array)
                .unwrap_or_default()
                .iter()
                .filter_map(Value::as_str)
                .collect()
        };
        let revision = match result.get("revision") {
            Some(Value::Number(revision)) => revision.as_str(),
            _ => "?",
        };
        let mut line = format!(
            "{}  revision {revision}",
            result.str_at("node").unwrap_or("?")
        );
        for (label, ids) in [("created", ids("created")), ("removed", ids("removed"))] {
            if !ids.is_empty() {
                line.push_str(&format!("  {label} {}", ids.join(" ")));
            }
        }
        say(&line);
    }
    Ok(())
}
