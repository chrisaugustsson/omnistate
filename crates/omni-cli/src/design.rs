//! `omni design read`: prints one board of a design doc in the tag format
//! (US-030). The project is found as for `omni changeset`.

use crate::args::Args;
use crate::changesets::{is_uuid, project_id};
use crate::commands::{agent, say};
use crate::components::is_node_id;
use crate::error::{CliError, Result, bail};
use crate::json::Value;

/// `omni design read --doc <id> --board <id> [--changeset <id> | --version <v>]`
pub fn read(args: &Args) -> Result<()> {
    const USAGE: &str =
        "omni design read --doc <id> --board <id> [--changeset <id> | --version <v>]";
    let doc = match args.doc.as_deref() {
        None => bail!("this command needs --doc <id>, the design doc: {USAGE}"),
        Some(doc) if is_uuid(doc) => doc.to_ascii_lowercase(),
        Some(doc) => {
            bail!("--doc {doc:?} is not a doc ID (a UUID); `omni doc show <doc>` takes it")
        }
    };
    let board = match args.board.as_deref() {
        None => bail!("this command needs --board <id>, the board to read: {USAGE}"),
        Some(board) if is_node_id(board) => board,
        Some(board) => bail!(
            "--board {board:?} is not a board's node ID (8 lowercase letters and digits, such as k3f9x2qa)"
        ),
    };
    if args.changeset.is_some() && args.doc_version.is_some() {
        bail!(
            "give --changeset or --version, not both: --changeset reads a draft, --version reads a saved version"
        );
    }
    let changeset = match args.changeset.as_deref() {
        None => None,
        Some(id) if is_uuid(id) => Some(id.to_ascii_lowercase()),
        Some(id) => {
            bail!("--changeset {id:?} is not a change set ID; `omni changeset list` shows them")
        }
    };
    let version = match args.doc_version.as_deref() {
        None => None,
        Some(version) if is_uuid(version) => Some(version.to_ascii_lowercase()),
        Some(version) => bail!("--version {version:?} is not a version ID (a UUID)"),
    };

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let base = format!("/api/v1/w/{}/projects/{project}", agent.workspace_id);
    let path = match (&changeset, &version) {
        (Some(changeset), _) => format!("{base}/changesets/{changeset}/docs/{doc}/boards/{board}"),
        (None, Some(version)) => format!("{base}/docs/{doc}/boards/{board}?version={version}"),
        (None, None) => format!("{base}/docs/{doc}/boards/{board}"),
    };
    let response = agent.request_explaining(
        "GET",
        &path,
        &[
            (
                404,
                "unknown_board",
                format!(
                    "the doc has no board {board}; a page, a frame and a text are no boards, and the tags of the page list its boards"
                ),
            ),
            (
                400,
                "not_a_design",
                format!("doc {doc} is not a design doc; `omni doc read` prints an html doc"),
            ),
            (
                409,
                "no_mainline_version",
                format!(
                    "doc {doc} has no mainline version yet, so it exists only in the change set that started it; read it with --changeset <id>"
                ),
            ),
        ],
    )?;
    let text = response.text();
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    let answer = Value::parse(&text).map_err(|_| {
        CliError::new(
            "the server's answer about the board is not readable; is it an OmniState API?",
        )
    })?;
    let Some(tags) = answer.str_at("tags") else {
        bail!("the server's answer has no tags for the board; is it an OmniState API?");
    };
    // The tags as the API gave them, and nothing else.
    say(tags);
    Ok(())
}
