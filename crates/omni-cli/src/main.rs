//! `omni`, the command line agents use to talk to the OmniState API.
//!
//! It is a single binary with no dependencies beyond the standard library, so it
//! starts in a millisecond or two. `--version` and `--help` answer before the
//! config file or the network are looked at.

mod args;
mod attach;
mod changesets;
mod claims;
mod commands;
mod components;
mod config;
mod docs;
mod error;
mod http;
mod identity;
mod json;
mod tokens;
mod toml;
mod versions;
mod wait;

use std::process::ExitCode;

pub const HELP: &str = "\
omni - the OmniState command line for agents

Usage:
  omni login                          log in, pick a workspace and store an API key
  omni key list                       list the workspace's API keys (asks for the password)
  omni key revoke <key-id>            revoke an API key (asks for the password)
  omni project create <name>          create a project
  omni project list                   list the projects
  omni changeset create --title <t>   create a change set [--description <d>] [--run <tag>]
  omni changeset list                 list the project's change sets
  omni changeset show <id>            show a change set with its status history
  omni changeset submit <id>          send an open change set to review
  omni changeset reopen <id>          reopen a rejected change set
  omni changeset abandon <id>         abandon a change set (it is kept, never deleted)
  omni changeset rebase <id>          replay a change set that needs a rebase onto the current
                                      mainline; with no conflicts it merges, else it is open
                                      again and each node changed on both sides is a conflict
  omni changeset conflicts <id>       list the change set's conflicts, with the node on the
                                      mainline side and on the change set's side
  omni changeset resolve <conflict-id> --note <text>
                                      resolve a conflict once the draft is fixed with `omni doc
                                      edit`; a change set cannot be submitted until every
                                      conflict is resolved
  omni doc create --changeset <id> --kind html|design|design-system --module prd|architecture|db|ui
                --title <text>        start a doc in a change set (its first version comes
                                      through the change set)
  omni doc open <doc> --changeset <id>
                                      make (or reuse) the change set's draft of a doc
  omni doc read <doc> [--changeset <id> | --version <v>]
                                      print the doc as HTML with data-node-id (data-rev too
                                      for a draft); without options, the mainline version
  omni doc show <doc>                 show the doc's metadata, mainline version, drafts, claims
  omni doc edit <doc> --changeset <id>
                                      apply a batch of operations (a JSON array, from
                                      --body-file or standard input) to the draft; prints
                                      each node's new revision and the IDs of new nodes
  omni doc remove <doc> --changeset <id>
                                      remove a doc in a change set: saves a version that
                                      marks it removed; the doc and its history stay
  omni version save <doc> --changeset <id>
                                      save the change set's draft of a doc as a version, an
                                      unchangeable snapshot; prints the version's ID
  omni version diff <a> <b>           list the nodes added, removed, changed and moved from
                                      version <a> to version <b> of a doc
  omni component usage <component>    list every board of the project's designs that uses a
                                      component, from the mainline versions: an instance in a
                                      board, in another component's tree, or a slot value
  omni tokens import <file> --changeset <id>
                                      replace the whole design token tree (DTCG JSON, with a
                                      value per mode) of the change set's draft of the design
                                      system, in one all-or-nothing batch
  omni tokens export [--changeset <id> | --version <v>] [--out <file>]
                                      print the design tokens as DTCG JSON: the mainline
                                      version, a change set's draft or a saved version
  omni tokens export --per-mode --out <dir>
                                      write one file per mode, <dir>/<mode>.tokens.json
  omni wait --changeset <id> --until approved|rejected|merged|needs_rebase [--timeout <s>]
                                      wait until the change set is approved (by the human),
                                      rejected, merged or needs a rebase; prints its status
  omni claim <node> --changeset <id>  claim a board or section of the change set's draft, so
                                      that no other agent changes it; a claim lasts 10 minutes
                                      after your last operation in the draft
  omni release <node> --changeset <id>
                                      end your claim on a board or section
  omni attach <file>                  upload a file (at most 20 MB) and print attachment:<id>,
                                      how docs, designs and posts refer to it
  omni whoami                         show the workspace and who the key acts as
  omni api <METHOD> <path>            send one request to the API and print the response
  omni config show                    show the effective settings (never the key)

Options (before or after the command):
  --as <run>/<role>     the agent identity of this run (fallback: OMNI_RUN, OMNI_ROLE)
  --model <name>        the model behind the agent (fallback: OMNI_MODEL)
  --harness <name>      the harness that runs the agent (fallback: OMNI_HARNESS)
  --server <url>        the API address (fallback: OMNI_SERVER, then the config file)
  --project <name|id>   the project to work in (fallback: project in .omni.toml)
  --body-file <path>    send this file as the request body (for a change set: its
                        description; for `doc edit`: the operations); - reads
                        standard input
  --note <text>         how a conflict was resolved (`omni changeset resolve`)
  --changeset <id>      the change set a doc, tokens, claim, release or wait command works in
  --version <v>         the version that `omni doc read` or `omni tokens export` reads
  --out <path>          where `omni tokens export` writes: a file, or with --per-mode a directory
  --per-mode            `omni tokens export`: one file per mode
  --until <status>      the status `omni wait` waits for
  --timeout <seconds>   how long `omni wait` waits; without it, as long as it takes
  --json                print the full JSON instead of short text
  -h, --help            show this help
  -V, --version         show the version

Output:
  Results go to standard output: short text by default, the API's JSON with --json.
  Prompts, notices (such as directives) and errors go to standard error.
  An error is one line saying what went wrong and how to fix it.

Files:
  ~/.config/omni/config.toml   written by `omni login` (mode 600): server, workspace_id, api_key, email
  .omni.toml                   in a project directory: project = \"name\", the default project there

Rejecting and approving are the human's, in the web app; an agent cannot do them.

Exit codes:
  0  success
  1  the command failed: usage, login, network or API error (the message says which)
  2  `omni wait` timed out before its condition was met
  3  `omni wait`: the condition can no longer be met (the change set is abandoned, or
     merged while you wait for rejected or needs_rebase)
";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("omni: {error}");
            ExitCode::from(error.code())
        }
    }
}

fn run() -> error::Result<()> {
    let args = args::parse(std::env::args().skip(1))?;
    if args.version {
        commands::say(&format!("omni {}", env!("CARGO_PKG_VERSION")));
        return Ok(());
    }
    if args.help {
        commands::say(HELP);
        return Ok(());
    }
    commands::run(&args)
}
