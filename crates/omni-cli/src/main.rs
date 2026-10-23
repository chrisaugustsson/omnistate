//! `omni`, the command line agents use to talk to the OmniState API.
//!
//! It is a single binary that talks to the API over plain HTTP with the
//! standard library, so it starts in a millisecond or two. Its only dependency
//! is the Penpot reader of `omni-penpot`, for `omni import penpot`. `--version`
//! and `--help` answer before the config file or the network are looked at.

mod args;
mod attach;
mod changesets;
mod claims;
mod commands;
mod comments;
mod components;
mod config;
mod decisions;
mod design;
mod docs;
mod error;
mod features;
mod http;
mod identity;
mod import;
mod json;
mod posts;
mod thread_wait;
mod threads;
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
  omni feature create <name>          create a feature in the project [--description <text>];
                                      prints its ID and name
  omni feature list                   list the project's features: ID, status and name
  omni feature show <name|id>         show a feature
  omni changeset create --title <t>   create a change set [--description <d>] [--run <tag>];
                                      in the feature of --feature or .omni.toml, or at project
                                      level with --global
  omni changeset list                 list the project's change sets: ID, status, scope (project
                                      or the feature's name) and title; --feature <name|id> or
                                      --global list one scope only
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
  omni doc create [--feature <name|id>] --kind html|design|design-system
                --pillar spec|ui|architecture|db|research --title <text>
                                      create a doc at the app level, or in a feature; it is
                                      readable at once, with no change set (--changeset <id>
                                      starts it in a change set instead)
  omni doc list                       list the project's docs: ID, scope (project or the
                                      feature's name), kind, pillar and title; --feature
                                      <name|id> or --global list one scope only; --pillar
                                      <pillar> lists one pillar only
  omni doc open <doc> --changeset <id>
                                      make (or reuse) the change set's draft of a doc
  omni doc read <doc> [--changeset <id> | --version <v>]
                                      print the doc as HTML with data-node-id (data-rev too
                                      for a draft); without options, the doc's head (a
                                      design's tags)
  omni doc show <doc>                 show the doc's metadata, mainline version, drafts, claims
  omni doc edit <doc> [--changeset <id>]
                                      apply a batch of operations (a JSON array, from
                                      --body-file or standard input) to the live doc, at
                                      once and all or nothing (--changeset <id> edits that
                                      change set's draft instead); prints each node's new
                                      revision and the IDs of new nodes
  omni doc remove <doc> --changeset <id>
                                      remove a doc in a change set: saves a version that
                                      marks it removed; the doc and its history stay
  omni version save <doc> --changeset <id>
                                      save the change set's draft of a doc as a version, an
                                      unchangeable snapshot; prints the version's ID
  omni version diff <a> <b>           list the nodes added, removed, changed and moved from
                                      version <a> to version <b> of a doc
  omni design read --doc <id> --board <id> [--changeset <id> | --version <v>]
                                      print one board of a design doc in the tag format (a
                                      rev on every tag for a draft); without options, the
                                      mainline version
  omni design edit <doc> [--changeset <id>] [--board <id>]
                                      apply a batch of operations (a JSON array, from
                                      --body-file or standard input) to the design's head, or
                                      with --changeset to the change set's draft: insert a
                                      subtree in the tag format (a new board under a page
                                      too), replace, set, move and delete, each with the rev
                                      you have; the server assigns the IDs of new nodes and
                                      prints them. A write stores only the boards it changes.
                                      With --board the batch must stay inside that board
  omni component usage <component>    list every board of the project's designs that uses a
                                      component, from the heads: an instance in a board, in
                                      another component's tree, or a slot value
  omni tokens import <file> [--changeset <id>]
                                      replace the whole design token tree (DTCG JSON, with a
                                      value per mode) of the design system's head, or with
                                      --changeset of the change set's draft, in one
                                      all-or-nothing batch
  omni tokens export [--changeset <id> | --version <v>] [--out <file>]
                                      print the design tokens as DTCG JSON: the design
                                      system's head, a change set's draft or a saved version
  omni tokens export --per-mode --out <dir>
                                      write one file per mode, <dir>/<mode>.tokens.json
  omni import penpot <file.penpot> --project <id> [--feature <name>]
                                      import a Penpot export (made with 'Export file + libraries')
                                      straight into the project, with no change set: the
                                      pages and boards as a design doc (in the feature's UI
                                      pillar with --feature), the components, design tokens
                                      (themes become modes), colors and typographies as the
                                      app-level design system, added to the one the project
                                      has, the images as attachments; prints the docs and a
                                      report of what could not be imported
  omni thread create --changeset <id> --question <text> --criteria-file <path> [--sealed]
                                      start a thread, a question to work out in a change set;
                                      each line of the criteria file is one criterion, and the
                                      first phase is open unless --sealed; prints its ID
  omni thread phase <thread> --open|--sealed
                                      start the thread's next phase, open or sealed; a thread
                                      keeps taking phases after its change set merged
  omni thread digest <thread>         list the posts you may read, one line each: type, author
                                      role, summary and post ID (a sealed phase hides the
                                      posts of others until you have posted in it)
  omni post <thread> --type <type> --summary <text> [--body-file <path>] [--attach <file>]...
            [--link <target>]... [--confidence <0-1>] [--replaces <post>]
                                      post to a thread: a note, finding, position, objection,
                                      support, question or verdict with a one-line summary
                                      (at most 200 characters) and a Markdown body, read from
                                      --body-file or standard input; prints the post's ID.
                                      Each --attach uploads a file first; a --link is
                                      post:<id>, attachment:<id>, node:<doc>/<address> or an
                                      http(s) address; --replaces names your earlier post in
                                      the thread. Posts are never edited or deleted. Only the
                                      human posts directives; one that is new to you comes
                                      with the answer to your next request in the change set
                                      and is printed on standard error
  omni post show <post>               print one whole post: type, author role, summary, ID and
                                      the Markdown body; a post you may not read yet is
                                      reported as not found
  omni decision propose <thread> --file <decision.json>
                                      propose the decision that ends a thread, from a JSON
                                      file (docs/formats/decision.md has the schema): the
                                      chosen direction, the options with their scores and why
                                      each lost, assumptions, open questions, dissent and the
                                      posts it relies on; prints its ID and status. A new
                                      proposal supersedes the thread's earlier decision. Only
                                      the human approves or rejects a decision, in the web app
  omni decision show <thread|decision>
                                      print a decision with its status and any rejection
                                      reason: the one with this ID, or the thread's current
                                      decision
  omni changeset link-decision <changeset> <decision>
                                      link a change set to a decision it carries out; the
                                      change set cannot be approved until every linked
                                      decision is
  omni comment list [--doc <id>] [--feature <name|id>]
                                      list the project's comments, oldest first: ID, doc, node
                                      address, author, text and the replies. A comment whose
                                      latest reply is an agent's says `waiting on you`; one
                                      whose node left the doc says `orphaned` and shows what
                                      the node held; the human deletes a comment when
                                      satisfied, in the web app.
                                      --doc narrows to one doc, --feature to the docs of one
                                      feature
  omni comment reply <comment> <text>
                                      reply to a comment; without <text> it is read from
                                      --body-file or standard input
  omni wait --changeset <id> --until approved|rejected|merged|needs_rebase [--timeout <s>]
                                      wait until the change set is approved (by the human),
                                      rejected, merged or needs a rebase; prints its status
  omni wait --thread <id> --until <condition> [--timeout <s>]
                                      wait on a thread; the conditions are
                                        posts:<n>            n different agent identities have
                                                             posted in the current phase (the
                                                             last one when the wait began)
                                        roles:<pattern>[,<pattern>...]
                                                             every role pattern has posted in
                                                             the current phase (* is a wildcard)
                                        phase-change         a later phase has started
                                        decision:approved    the human approved the decision
                                        decision:rejected    the human rejected the decision
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
  --feature <name|id>   the feature that `omni changeset create` opens a change set in
                        (fallback: feature in .omni.toml); `omni changeset list` shows its
                        change sets or docs only
  --global              `omni changeset create` opens a project-level change set, whatever
                        .omni.toml says; `omni changeset list` and `omni doc list` show those only
  --body-file <path>    send this file as the request body (for a change set: its
                        description; for `doc edit` and `design edit`: the operations); -
                        reads standard input
  --question <text>     the question that `omni thread create` starts a thread with
  --criteria-file <path>
                        the file with the thread's criteria, one per line
  --open, --sealed      whether a thread's first phase (`--sealed` only) or its next
                        phase (`omni thread phase`) is open or sealed
  --type <type>         the type of the post that `omni post` makes
  --summary <text>      the one-line summary of the post
  --attach <file>       a file that `omni post` uploads and attaches (repeat for more)
  --link <target>       what a post points to: post:<id>, attachment:<id>,
                        node:<doc>/<address> or an http(s) address (repeat for more)
  --confidence <n>      how sure the post is, from 0 to 1
  --replaces <post>     your earlier post in the thread that this one replaces
  --file <path>         the decision file that `omni decision propose` sends
  --note <text>         how a conflict was resolved (`omni changeset resolve`)
  --changeset <id>      the change set a doc, design edit, tokens, claim, release or wait command works in
  --doc <id>            the design doc that `omni design read` reads or `omni design edit` edits
                        (`omni design edit` also takes it as an argument), or the doc whose
                        comments `omni comment list` shows
  --board <id>          the board of that doc that `omni design read` prints, or that a
                        `omni design edit` batch stays inside
  --version <v>         the version that `omni doc read`, `omni design read` or
                        `omni tokens export` reads
  --out <path>          where `omni tokens export` writes: a file, or with --per-mode a directory
  --per-mode            `omni tokens export`: one file per mode
  --until <status>      the status or condition `omni wait` waits for
  --thread <id>         the thread that `omni wait` waits on, instead of a change set
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
  .omni.toml                   in a project directory: project = \"name\", the default project there,
                               and optionally feature = \"name\", the default feature there

Rejecting and approving are the human's, in the web app; an agent cannot do them.

Exit codes:
  0  success
  1  the command failed: usage, login, network or API error (the message says which)
  2  `omni wait` timed out before its condition was met
  3  `omni wait`: the condition can no longer be met (the change set is abandoned, or
     merged while you wait for rejected or needs_rebase; for a thread, a later phase has
     started while posts: or roles: is not met, or the decision has the other status)
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
