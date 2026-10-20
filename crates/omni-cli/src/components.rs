//! `omni component usage <component>`: lists the boards of the project's design
//! docs that use a component (US-029). The project is found as for `omni
//! changeset`.

use crate::args::Args;
use crate::changesets::project_id;
use crate::commands::{Agent, agent, say};
use crate::error::{CliError, Result, bail};
use crate::json::Value;

/// The alphabet of node IDs: Crockford's base32 in lowercase.
const NODE_ID_ALPHABET: &str = "0123456789abcdefghjkmnpqrstvwxyz";

fn is_node_id(text: &str) -> bool {
    text.len() == 8 && text.chars().all(|c| NODE_ID_ALPHABET.contains(c))
}

fn project_path(agent: &Agent, project: &str) -> String {
    format!("/api/v1/w/{}/projects/{project}", agent.workspace_id)
}

/// `omni component usage <component>`
pub fn usage(args: &Args, component: Option<&str>) -> Result<()> {
    const USAGE: &str = "omni component usage <component>";
    let Some(component) = component else {
        bail!("this command needs the component's node ID: {USAGE}");
    };
    if !is_node_id(component) {
        bail!(
            "{component:?} is not a component's node ID (8 lowercase letters and digits, such as k3f9x2qa): {USAGE}"
        );
    }
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let response = agent.request_naming(
        "GET",
        &format!(
            "{}/components/usage?component={component}",
            project_path(&agent, &project)
        ),
        "unknown_component",
        &format!(
            "the design system does not define a component {component}; its components are in the design system's draft or mainline version"
        ),
    )?;
    let text = response.text();
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    let answer = Value::parse(&text).map_err(|_| {
        CliError::new(
            "the server's answer about the component's usage is not readable; is it an OmniState API?",
        )
    })?;
    let boards = answer
        .get("boards")
        .and_then(Value::as_array)
        .unwrap_or_default();
    if boards.is_empty() {
        say(&format!("no board uses the component {component}"));
        return Ok(());
    }
    say(&format!(
        "{} {} the component {component}:",
        boards.len(),
        if boards.len() == 1 {
            "board uses"
        } else {
            "boards use"
        }
    ));
    for board in boards {
        say(&format!(
            "  {} ({}) in {}, page {}",
            board.str_at("board_name").unwrap_or("?"),
            board.str_at("board_id").unwrap_or("?"),
            board.str_at("doc_title").unwrap_or("?"),
            board.str_at("page_name").unwrap_or("?"),
        ));
    }
    Ok(())
}
