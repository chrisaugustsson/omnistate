//! `omni`, the command line agents use to talk to the OmniState API.
//!
//! It is a single binary with no dependencies beyond the standard library, so it
//! starts in a millisecond or two. `--version` and `--help` answer before the
//! config file or the network are looked at.

mod args;
mod attach;
mod changesets;
mod commands;
mod config;
mod docs;
mod error;
mod http;
mod identity;
mod json;
mod toml;

use std::process::ExitCode;

/// Exit codes of `omni`; `HELP` documents them.
const FAILURE: u8 = 1;

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
  --changeset <id>      the change set a doc command works in
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
  3  `omni wait`: the condition can no longer be met
";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("omni: {error}");
            ExitCode::from(FAILURE)
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
