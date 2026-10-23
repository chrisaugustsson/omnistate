//! `omni design read` prints one board of a design doc in the tag format
//! (US-030), and `omni design edit` applies a batch of operations to a design:
//! to its head without a change set (US-104), or to a change set's draft with
//! one (US-031). The project is found as for `omni changeset`.

use crate::args::Args;
use crate::changesets::{is_uuid, project_id};
use crate::commands::{agent, say};
use crate::components::is_node_id;
use crate::docs::{print_edit, read_batch};
use crate::error::{CliError, Result, bail};
use crate::json::{Value, quote};

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
        None,
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

/// `omni design edit <doc> [--changeset <id>] [--board <id>] [--body-file <path|->]`:
/// without `--changeset` the batch applies to the design's head, with it to the
/// change set's draft. The doc is the argument or `--doc`.
pub fn edit(args: &Args, positional: Option<&str>) -> Result<()> {
    const USAGE: &str =
        "omni design edit <doc> [--changeset <id>] [--board <id>] [--body-file <path|->]";
    let changeset = match args.changeset.as_deref() {
        None => None,
        Some(id) if is_uuid(id) => Some(id.to_ascii_lowercase()),
        Some(id) => {
            bail!("--changeset {id:?} is not a change set ID; `omni changeset list` shows them")
        }
    };
    let doc = match (positional, args.doc.as_deref()) {
        (Some(doc), Some(flag)) if doc != flag => {
            bail!("the doc is given twice, as {doc:?} and --doc {flag:?}: give it once")
        }
        (None, None) => {
            bail!("this command needs the design doc, as an argument or --doc <id>: {USAGE}")
        }
        (Some(doc), _) | (None, Some(doc)) if is_uuid(doc) => doc.to_ascii_lowercase(),
        (Some(doc), _) | (None, Some(doc)) => {
            bail!("{doc:?} is not a doc ID (a UUID); `omni doc list` shows them")
        }
    };
    let board = match args.board.as_deref() {
        None => None,
        Some(board) if is_node_id(board) => Some(board),
        Some(board) => bail!(
            "--board {board:?} is not a board's node ID (8 lowercase letters and digits, such as k3f9x2qa)"
        ),
    };
    let batch = read_batch(args)?;

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let mut body = format!("{{\"ops\":{}", batch.trim());
    if let Some(board) = board {
        body.push_str(&format!(",\"board\":{}", quote(board)));
    }
    body.push('}');
    let unknown_board = board.map(|board| {
        (
            404,
            "unknown_board",
            format!(
                "the doc has no board {board}; a page, a frame and a text are no boards, and the tags of the page list its boards"
            ),
        )
    });
    let explanations: Vec<_> = unknown_board.into_iter().collect();
    let base = format!("/api/v1/w/{}/projects/{project}", agent.workspace_id);
    let path = match &changeset {
        Some(changeset) => format!("{base}/changesets/{changeset}/docs/{doc}/ops"),
        None => format!("{base}/docs/{doc}/ops"),
    };
    let response = agent.request_explaining("POST", &path, Some(body.as_bytes()), &explanations)?;
    print_edit(args, &response.text())
}
