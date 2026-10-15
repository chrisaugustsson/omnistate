//! `omni claim` and `omni release`: take and end a claim on a board or section of
//! a change set's draft. While the claim is held, no other agent identity can
//! claim or change that subtree. The project is found as for `omni changeset`.

use crate::args::Args;
use crate::changesets::{is_uuid, project_id};
use crate::commands::{agent, say};
use crate::error::{CliError, Result, bail};
use crate::json::{Value, quote};

/// A node ID as it goes into a request: 8 lowercase letters and digits. The
/// server knows which of those are IDs.
fn node_id(node: Option<&str>, usage: &str) -> Result<String> {
    match node {
        None => bail!("this command needs the node's ID: {usage}"),
        Some(node)
            if node.len() == 8
                && node
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) =>
        {
            Ok(node.to_string())
        }
        Some(node) => bail!("{node:?} is not a node ID (8 lowercase letters and digits): {usage}"),
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

/// `omni claim <node> --changeset <id>`
pub fn take(args: &Args, node: Option<&str>) -> Result<()> {
    const USAGE: &str = "omni claim <node> --changeset <id>";
    let node = node_id(node, USAGE)?;
    let changeset = changeset_option(args, USAGE)?;
    let not_found = format!(
        "not_found: no draft of change set {changeset} has the node {node}; `omni doc read <doc> --changeset <id>` shows the node IDs of a draft"
    );
    send(args, &changeset, "claims", &node, &not_found, |claim| {
        format!(
            "claimed {node}  until {}",
            claim.str_at("expires_at").unwrap_or("?")
        )
    })
}

/// `omni release <node> --changeset <id>`
pub fn release(args: &Args, node: Option<&str>) -> Result<()> {
    const USAGE: &str = "omni release <node> --changeset <id>";
    let node = node_id(node, USAGE)?;
    let changeset = changeset_option(args, USAGE)?;
    let not_found = format!(
        "not_found: you hold no claim on the node {node} in change set {changeset}; `omni doc show <doc>` lists the claims"
    );
    send(
        args,
        &changeset,
        "claims/release",
        &node,
        &not_found,
        |_| format!("released {node}"),
    )
}

fn send(
    args: &Args,
    changeset: &str,
    route: &str,
    node: &str,
    not_found: &str,
    line: impl Fn(&Value) -> String,
) -> Result<()> {
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let body = format!("{{\"node\":{}}}", quote(node));
    let response = agent.request_or_not_found(
        "POST",
        &format!(
            "/api/v1/w/{}/projects/{project}/changesets/{changeset}/{route}",
            agent.workspace_id
        ),
        Some(body.as_bytes()),
        not_found,
    )?;
    let text = response.text();
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    let answer = Value::parse(&text).map_err(|_| {
        CliError::new(
            "the server's answer about the claim is not readable; is it an OmniState API?",
        )
    })?;
    say(&line(answer.get("claim").unwrap_or(&answer)));
    Ok(())
}
