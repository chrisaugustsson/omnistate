//! `omni`, the command line agents use to talk to the OmniState API.
//!
//! It is a single binary that talks to the API over plain HTTP with the
//! standard library, so it starts in a millisecond or two. Its only dependency
//! is the Penpot reader of `omni-penpot`, for `omni import penpot`. `--version`
//! and `--help` answer before the config file or the network are looked at.

mod args;
mod attach;
mod commands;
mod comments;
mod components;
mod config;
mod design;
mod docs;
mod error;
mod export;
mod features;
mod http;
mod identity;
mod import;
mod json;
mod links;
mod posts;
mod projects;
mod stories;
mod thread_wait;
mod threads;
mod tokens;
mod toml;

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
  omni feature list                   list the project's features: ID and name
  omni feature show <name|id>         show a feature
  omni feature rename <name|id> <new name>
                                      rename a feature; its ID, docs and threads stay
  omni feature delete <name|id>       delete a feature with its docs, threads and comments (they
                                      are kept, hidden; the name can be used again)
  omni doc create [--feature <name|id>] --kind html|design|design-system
                --pillar spec|ui|architecture|db|research --title <text>
                                      create a doc at the app level, or in a feature; it is
                                      readable at once
  omni doc list                       list the project's docs: ID, scope (project or the
                                      feature's name), kind, pillar and title; --feature
                                      <name|id> or --global list one scope only; --pillar
                                      <pillar> lists one pillar only
  omni doc read <doc>                 print the doc's head: HTML with data-omni-id and
                                      data-rev (a design's tags)
  omni doc show <doc>                 show the doc's metadata
  omni doc edit <doc>                 apply a batch of operations (a JSON array, from
                                      --body-file or standard input) to the doc, at once and
                                      all or nothing; prints each node's new revision and the
                                      IDs of new nodes
  omni doc rename <doc> <title>       rename a doc; its ID, head and comments stay
  omni doc delete <doc>               delete a doc (it is kept, hidden from every list and
                                      read; the deletion is logged)
  omni doc activity <doc> [--limit <n>] [--before <seq>]
                                      list what was written to a doc, newest first: number,
                                      time, the writing agent's role and a one-line summary.
                                      --limit (1 to 200, 50 by default) and --before page
                                      back through the older entries
  omni design read --doc <id> --board <id>
                                      print one board of a design doc's head in the tag format
  omni design edit <doc> [--board <id>]
                                      apply a batch of operations (a JSON array, from
                                      --body-file or standard input) to the design's head:
                                      insert a subtree in the tag format (a new board under a
                                      page too), replace, set, move and delete, each with the
                                      rev you have; the server assigns the IDs of new nodes
                                      and prints them. A write stores only the boards it
                                      changes. With --board the batch must stay inside that
                                      board
  omni component usage <component>    list every board of the project's designs that uses a
                                      component, from the heads: an instance in a board, in
                                      another component's tree, or a slot value
  omni tokens import <file>           replace the whole design token tree (DTCG JSON, with a
                                      value per mode) of the design system's head, in one
                                      all-or-nothing batch
  omni tokens export [--out <file>]   print the design tokens of the design system's head as
                                      DTCG JSON
  omni tokens export --per-mode --out <dir>
                                      write one file per mode, <dir>/<mode>.tokens.json
  omni export --project <name|id> [--feature <name|id>] --out <dir>
                                      write the scope's plan to a folder: manifest.json, the
                                      HTML docs by pillar, each design board in the tag format
                                      under ui/<doc>/, and the design system's tokens as
                                      tokens.json and tokens.css (variables per mode). The
                                      app level without --feature
  omni import penpot <file.penpot> --project <id> [--feature <name>]
                                      import a Penpot export (made with 'Export file + libraries')
                                      straight into the project: the pages and boards as a design doc (in the feature's UI
                                      pillar with --feature), the components, design tokens
                                      (themes become modes), colors and typographies as the
                                      app-level design system, added to the one the project
                                      has, the images as attachments; prints the docs and a
                                      report of what could not be imported
  omni thread create <question> [--feature <name|id>] [--sealed]
                                      start a thread, a question that agents work out together,
                                      at the app level or in a feature; with --sealed an
                                      agent reads no other agent's posts until it has posted
                                      itself; prints its ID
  omni thread list                    list the project's threads, of every scope: ID, scope,
                                      whether it is sealed or concluded, and the question
  omni thread read <thread>           print the question, the posts you may read (a sealed
                                      thread hides the posts of others until you have posted)
                                      and the conclusion when there is one
  omni thread conclude <thread> <text>
                                      write the thread's conclusion; the thread keeps taking
                                      posts
  omni thread digest <thread>         list the posts you may read, one line each: author role,
                                      summary and post ID
  omni post <thread> --summary <text> [<body>] [--body-file <path>] [--attach <file>]...
            [--link <target>]... [--confidence <0-1>] [--replaces <post>]
                                      post to a thread with a one-line summary (at most 200
                                      characters) and a Markdown body, given as the argument,
                                      else read from --body-file or standard input; prints the
                                      post's ID. Each --attach uploads a file first; a --link
                                      is post:<id>, attachment:<id>, node:<doc>/<address> or
                                      an http(s) address; --replaces names your earlier post in
                                      the thread. Posts are never edited or deleted
  omni post show <post>               print one whole post: author role, summary, ID and the
                                      Markdown body; a post you may not read yet is reported
                                      as not found
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
  omni story add <file> [--priority <0-3>] [--link <doc>:<node>[:<label>]]... [--number-from <n>]
                                      save a story written in the prd format (### US-<n>: title,
                                      Description, Topic, Interfaces, Files, Blocked by,
                                      Acceptance Criteria) in the feature; the server numbers it;
                                      prints its key
  omni story list [--status <status>] [--pickable]
                                      list the feature's stories: key, priority, status and title;
                                      --pickable lists those an implementer may take
  omni story read <key|id>            print a story: text, criteria, blockers, holder and the
                                      open findings
  omni story edit <key> [--priority <n>] [--blocked-by US-1,US-2] [--text <file>] [--link <doc>:<node>[:<label>]]...
                                      change a story; the text comes from the file
  omni story cancel <key>             cancel a story
  omni story claim <key> --run <id> --stage <stage>
                                      hold a story for a run, at implementing, review or testing;
                                      a claim is taken only from the stage's waiting state
                                      (backlog, implemented, reviewed), else 409 not_waiting
  omni story release <key> --run <id>
                                      let go of a story, whoever holds it; a working story returns
                                      to its waiting state
  omni story move <key> --run <id> --to <column>
                                      move the story you hold to backlog, implementing, implemented,
                                      review, reviewed, testing or done
  omni story block <key> --run <id>   flag the story as waiting for the human and return a working
                                      story to its waiting state; unblock clears the flag
  omni story tick <key> --run <id> --criterion <n> [--pending] [--unticked]
                                      tick a criterion (0-based); --pending also marks it for the
                                      human to check; --unticked takes the tick away
  omni story finding <key> --run <id> --stage <s> --text <t> [--file <path>]...
                                      record a review or test finding
  omni story resolve <key> --run <id> --finding <n>
                                      mark finding n (0-based) resolved
  omni story decision <key> --run <id> --question <q> --choice <c> --by <agent|human>
                                      record a decision
  omni story files <key> --run <id> <path>...
                                      set the files the story changed
                                      Each write reads the story first and sends its version; a
                                      stale one is a conflict (exit 3).
  omni wait --thread <id> --posts <n> [--concluded] [--timeout <s>]
                                      wait until the thread has n posts that you may read (a
                                      sealed thread counts none of the others' posts until you
                                      have posted) and, with --concluded, a conclusion
  omni attach <file>                  upload a file (at most 20 MB) and print attachment:<id>,
                                      how docs, designs and posts refer to it
  omni checks [--json]                print the fast, stack_up and stack commands of the [checks]
                                      table in .omni.toml
  omni whoami                         show the workspace and who the key acts as
  omni api <METHOD> <path>            send one request to the API and print the response
  omni config show                    show the effective settings (never the key)

Options (before or after the command):
  --as <run>/<role>     the agent identity of this run (fallback: OMNI_RUN, OMNI_ROLE)
  --model <name>        the model behind the agent (fallback: OMNI_MODEL)
  --harness <name>      the harness that runs the agent (fallback: OMNI_HARNESS)
  --server <url>        the API address (fallback: OMNI_SERVER, then the config file)
  --project <name|id>   the project to work in (fallback: OMNI_PROJECT, then the folder's link)
  --feature <name|id>   the feature that `omni doc create` creates a doc in and `omni doc
                        list` shows (fallback: OMNI_FEATURE)
  --global              `omni doc create` creates an app-level doc, whatever OMNI_FEATURE says;
                        `omni doc list` shows those only
  --body-file <path>    send this file as the request body (for `doc edit` and `design
                        edit`: the operations); - reads standard input
  --sealed              `omni thread create`: a sealed thread hides the posts of other agents
                        from an agent until it has posted
  --summary <text>      the one-line summary of the post
  --attach <file>       a file that `omni post` uploads and attaches (repeat for more)
  --link <target>       what a post points to: post:<id>, attachment:<id>,
                        node:<doc>/<address> or an http(s) address (repeat for more)
  --confidence <n>      how sure the post is, from 0 to 1
  --replaces <post>     your earlier post in the thread that this one replaces
  --doc <id>            the design doc that `omni design read` reads or `omni design edit` edits
                        (`omni design edit` also takes it as an argument), or the doc whose
                        comments `omni comment list` shows
  --board <id>          the board of that doc that `omni design read` prints, or that a
                        `omni design edit` batch stays inside
  --out <path>          where `omni tokens export` writes: a file, or with --per-mode a directory;
                        the folder `omni export` writes to
  --per-mode            `omni tokens export`: one file per mode
  --thread <id>         the thread that `omni wait` waits on
  --posts <n>           `omni wait --thread`: wait for this many posts that you may read
  --concluded           `omni wait --thread`: wait for the thread's conclusion
  --timeout <seconds>   how long `omni wait` waits; without it, as long as it takes
  --priority <n>        `omni story add`: the story's priority, 0 to 3 (3 by default)
  --number-from <n>     `omni story add`: the number the project's first story starts after
  --status <status>     `omni story list`: only the stories in this status
  --pickable            `omni story list`: only the stories that can be picked
  --json                print the full JSON instead of short text
  -h, --help            show this help
  -V, --version         show the version

Output:
  Results go to standard output: short text by default, the API's JSON with --json.
  Prompts, notices and errors go to standard error.
  An error is one line saying what went wrong and how to fix it.

Files:
  ~/.config/omni/config.toml   written by `omni login` (mode 600): server, workspace_id, api_key, email
  .omni.toml                   in a project directory: the [checks] table

Exit codes:
  0  success
  1  the command failed: usage, login, network or API error (the message says which)
  2  `omni wait` timed out before its condition was met
  3  `omni story` write refused as a conflict: the story changed, read it again
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
