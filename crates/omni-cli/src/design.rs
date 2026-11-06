//! `omni design read` prints one board of a design doc in the tag format
//! (US-030), and `omni design edit` applies a batch of operations to the head
//! of a design (US-104), and `omni design flows` lists the prototype graph of a
//! page (US-22). The project is found as for `omni project`.

mod graph;

use crate::args::Args;
use crate::commands::{agent, checked, say};
use crate::components::is_node_id;
use crate::docs::{print_edit, read_batch};
use crate::error::{CliError, Result, bail};
use crate::json::{Value, quote};
use crate::projects::{is_uuid, project_id};
use graph::Graph;

/// `omni design read --doc <id> --board <id>`
pub fn read(args: &Args) -> Result<()> {
    const USAGE: &str = "omni design read --doc <id> --board <id>";
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

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let path = format!(
        "/api/v1/w/{}/projects/{project}/docs/{doc}/boards/{board}",
        agent.workspace_id
    );
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

/// `omni design edit <doc> [--board <id>] [--body-file <path|->]`: the batch
/// applies to the design's head. The doc is the argument or `--doc`.
pub fn edit(args: &Args, positional: Option<&str>) -> Result<()> {
    const USAGE: &str = "omni design edit <doc> [--board <id>] [--body-file <path|->]";
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
    let path = format!("{base}/docs/{doc}/ops");
    let response = agent.request_explaining("POST", &path, Some(body.as_bytes()), &explanations)?;
    print_edit(args, &response.text())
}

/// `omni design flows --doc <id> [--page <id>]`: the flows of each page, every
/// board's interactions, the boards nothing reaches and the dangling targets.
pub fn flows(args: &Args) -> Result<()> {
    const USAGE: &str = "omni design flows --doc <id> [--page <id>]";
    let doc = match args.doc.as_deref() {
        None => bail!("this command needs --doc <id>, the design doc: {USAGE}"),
        Some(doc) if is_uuid(doc) => doc.to_ascii_lowercase(),
        Some(doc) => {
            bail!("--doc {doc:?} is not a doc ID (a UUID); `omni doc list` shows them")
        }
    };
    let page = args.page.as_deref();

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let doc_path = format!(
        "/api/v1/w/{}/projects/{project}/docs/{doc}",
        agent.workspace_id
    );
    let not_a_design = || CliError::with_code(2, format!("{doc} is not a design"));
    let response = agent.request_unchecked("GET", &format!("{doc_path}/html?view=outline"))?;
    if response.status == 400 {
        return Err(not_a_design());
    }
    let response = checked(response)?;
    let head = Value::parse(&response.text()).map_err(|_| {
        CliError::new(
            "the server's answer about the design is not readable; is it an OmniState API?",
        )
    })?;
    let Some(pages) = head.get("outline").and_then(Value::as_array) else {
        return Err(not_a_design());
    };
    let shown: Vec<&Value> = match page {
        None => pages.iter().collect(),
        Some(page) => match pages.iter().find(|p| p.str_at("id") == Some(page)) {
            Some(found) => vec![found],
            None => {
                return Err(CliError::with_code(
                    2,
                    format!("{page} is not a page of the design"),
                ));
            }
        },
    };

    let mut blocks = Vec::new();
    for page in shown {
        let mut boards = Vec::new();
        for board in page
            .get("children")
            .and_then(Value::as_array)
            .unwrap_or_default()
        {
            let id = board.str_at("id").unwrap_or_default();
            if !is_node_id(id) {
                bail!("the server named a board {id:?}; is it an OmniState API?");
            }
            let answer = agent.request_explaining(
                "GET",
                &format!("{doc_path}/boards/{id}"),
                None,
                &[(
                    400,
                    "not_a_design",
                    format!("doc {doc} is not a design doc"),
                )],
            )?;
            let answer = Value::parse(&answer.text()).map_err(|_| {
                CliError::new(
                    "the server's answer about the board is not readable; is it an OmniState API?",
                )
            })?;
            let Some(node) = answer.get("node") else {
                bail!(
                    "the server's answer has no node for the board {id}; is it an OmniState API?"
                );
            };
            boards.push(node.clone());
        }
        blocks.push(Graph::from(page, &boards).in_doc(pages).render());
    }
    say(blocks.join("\n").trim_end());
    Ok(())
}
