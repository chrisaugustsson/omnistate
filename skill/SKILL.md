---
name: omni
description: OmniState, the shaping space where a human and agents work out what an app, its parts and their features should be, through the `omni` CLI. Use to shape a new track of work (spec, designs, architecture, db, research), fold a finished track into the app level, document an existing app from its codebase, read and answer the human's comments, or run a thread.
---

OmniState holds everything about what an app, its parts and their features are meant to be, while they are being shaped. It is a work environment: it never syncs with code and may drift, and it is not a task tracker. You write to it directly with `omni`; the human reads it in the browser and answers with comments.

## The model

- **Workspace > project > part > feature.** A project has one **app level** (the product as intended now) and **parts**, each an app or a service (`omni part`). A part has **features**: what it can do, one capability each (`omni feature`).
- **Tracks are the work.** A **track** is one piece of work on one part: the change it brings, with its own docs, behaviour tree, threads and board of tasks (`omni track`). It has a name, a description, an owner and a state (`planning`, `building`, `done`, `dropped`), and says which features of its part it changes (`omni track link-feature`).
- **Five pillars per scope**, the same at the app level, on a part, on a feature and in every track: `spec`, `ui`, `architecture`, `db`, `research`. Each pillar holds any number of docs. Any scope and any pillar is yours to write.
- **Docs are live.** A write lands at once in the head, which is the only version; every read returns it. No drafts, no approvals, no review step: the human sees what you wrote the moment you wrote it.
  - `html` docs (any pillar): free HTML and CSS. See [html-docs.md](html-docs.md).
  - `design` docs (`ui` only): pages and boards in a JSX-like tag format. See [designs.md](designs.md).
  - `design-system` (`ui`, app level only, one per project): the design tokens and components that designs refer to. See [designs.md](designs.md).
- **Nodes.** A block of an HTML doc and a layer of a design is a node with an 8-character ID. Every node has a revision; an edit of an existing node states the revision you read, and a stale one is refused with a conflict (nothing changes), so read again and resend.
- **Comments** are the human's pointer: one comment sits on one node of one doc (a node inside a component instance is `<instanceId>/<componentNodeId>`). It has text, replies and, when its node has since been removed, the mark `orphaned` with a snapshot of what the node held. A comment has no status. You reply; the human deletes it when satisfied. You cannot delete.
- **Threads** are for questions that agents or the human work out in posts: a question, posts, an optional conclusion. See [threads.md](threads.md).
- **Behaviour tree.** A track has one: what the product does in each situation, kept as records that `omni behaviour` reads and writes. See [behaviour.md](behaviour.md).

## Start

1. `omni whoami --as <run>/<role>`. When it says `not logged in`, ask the human to run `omni login` in their terminal, then continue.
2. Pass `--as <run>/<role>` on every command, with a short run name and role you keep for the whole session (`--as shaping-1/shaper`); shell variables do not survive between Bash calls. The server records it as the author of your writes, comments and posts.
3. The project is, in this order: `--project <name|id>`, `OMNI_PROJECT`, or the folder's link made with `omni link <project>` (`omni link` alone shows the link in effect; `omni unlink` removes it). The track is `--track <name|id>` or `OMNI_TRACK`. `omni project list` shows the projects, `omni project create <name>` makes one; `omni part list`, `omni feature list` and `omni track list` show the rest.
4. `omni --help` lists every command and flag.

### Pick the part

Every track belongs to one part. To pick it for a new track:

1. `omni link --json` in the folder. When its `parts` names exactly one part, that is the part.
2. Otherwise (none, or several) ask the human which part the track is on, listing `omni part list`, with the one you recommend first. A project with no parts yet gets one first: `omni part create "<name>" --kind app|service`, named with the human, then `omni link "<project>" --part "<name>"` in its folder.

## The main loop

Run it whenever the human asks you to read the comments, and after finishing a recipe.

1. `omni comment list --as ...` (add `--track <name|id>` or `--doc <id>` to narrow). **Open** comments are the ones without the mark `waiting on you`, orphaned ones included. That mark means your reply is the latest and the human has not answered; leave those until they do.
2. For each open comment, read the node it names (`omni doc read <doc> --node <id>`, or `omni design read --doc <doc> --board <board>`), work out what the human asks, and act: edit the doc, or start a thread when the answer needs a decision.
3. Reply with what you did or decided: `omni comment reply <comment> "<text>"`. For an orphaned comment, say where its content went or why it is gone.
4. Run `omni comment list` again. You are done when every comment shows `waiting on you`.

## Recipes

Give the human the link to what you wrote when you finish a step: `<server>/w/<workspace_id>/projects/<project_id>/docs/<doc_id>` (and `/tracks/<track_id>` for a track; `omni config show` has the server and workspace, and the IDs are in the output of `omni project list`, `track list` and `doc list`). Then wait for comments.

### Shape a new track

1. Pick the part (see [Pick the part](#pick-the-part)), then `omni track create "<name>" --part "<part>" --description "<one or two sentences: what changes for whom>"`. When the track changes features the part already has, `omni track link-feature "<name>" "<feature>"` for each.
2. Spec: `omni doc create --track "<name>" --kind html --pillar spec --title "Spec"`, then fill it (see [html-docs.md](html-docs.md)). Cover why, what the user can do, rules and edge cases, and an open-questions list.
3. UI: `omni doc create --track "<name>" --kind design --pillar ui --title "<screens>"`, then write one board per screen or state (see [designs.md](designs.md)). Read the app level first (`omni doc list --global`, and `omni tokens export` when it has a design system) and build on what exists. With interactions and flows in the design, `omni design flows --doc <doc>` lists the prototype graph: flows, interactions, unreachable boards and dangling targets.
4. `architecture`, `db` and `research` docs when the track touches them: the same `doc create --track ... --pillar <pillar>` with `--kind html`.
5. A question the human must decide goes in `omni thread create "<question>" --track "<name>"`; say which in your hand-off.
6. Hand off with the links, then run the main loop when comments arrive.

Done when the track has a Spec doc with written content, a design with at least one board of nodes, and the human has the links.

### Prototype a flow

1. Write the boards of the flow in a design doc (see [designs.md](designs.md)) and read their IDs back with `omni design read`.
2. On each layer that moves the person on, set `interactions` with a `dest` that is a board ID (`## Prototyping` in [designs.md](designs.md)). State that changes along the way is a token in a collection, changed by `setVariable` and bound with `$path` (`## Variables`).
3. Set `flows` on the page, one flow per entry point, each `start` a board ID.
4. Run `omni design flows --doc <doc>` and fix what it reports: a dangling target, a board nothing reaches.

Done when `omni design flows` reports no dangling targets and no unreached boards except the flow starts.

### Shape a track for the board

For a track that will be built story by story from the board. Three tools carry it, installed by the human with `make install-agent-tools`.

1. `omni-grill`: the grill session. It writes the Spec doc as the decisions are made.
2. `omni-stories`: splits the track into tasks (story records) on its board, with criteria from the behaviour tree when the track has one.
3. `omni-deliver`: the workflow that runs the board, implementing the stories in worktrees, reviewing and testing them.

A story's status is one of eight: `backlog`, `implementing`, `implemented`, `review`, `reviewed`, `testing`, `done`, `cancelled`.
- `implemented` is a waiting status: the implementer has finished and the story waits for a reviewer.
- `reviewed` is a waiting status: the review passed and the story waits for the test lane.
`omni story claim` takes a story only from the waiting state of the stage asked for (backlog for implementing, implemented for review, reviewed for testing); `release` and `block` return a working story to its waiting state.

Start with `omni-grill`; run the next one when the human confirms the step before it.

Done when the human has the Spec doc, and the stories are on the board or the human chose not to cut them.

### Fold a finished track into the app level

Only when the human asks. The app level should read as the app with this track's change in it.

1. `omni doc list --track "<name>"` and `omni doc list --global`: pair each track doc with the app-level doc of the same pillar.
2. Per pair, read both and edit the app-level doc so it describes the app as it now is (`replace` or `set` the nodes that change, `insert` new sections). Do not copy the track's wording about "adding"; write the result. When the app level has no doc for that pillar, create one with `omni doc create --global ...` and write it from the track's.
3. A design: read the track's boards (`omni design read`) and insert them, as tags without `id`, into the app-level UI design doc (or create it), replacing the board of any screen they supersede.
4. Thread conclusions that settle a rule or a decision go into the app-level spec or architecture doc.
5. Leave the track in place and move it to done when the human says so (`omni track state "<name>" done`); `omni track delete` hides its docs, threads and comments, so run it only when the human asks.

Done when every decision, rule, screen and schema change in the track's docs is in an app-level doc of the same pillar.

### Document an existing app from its codebase

You write the app-level docs (`--global`, the default without `--track` or `OMNI_TRACK`) from the code.

1. Ask the human once: is there a `.penpot` file of the app's design (Penpot: Export file + libraries)? Wait for the answer.
2. Read the codebase: entry points, routes and screens, modules and services, data models and migrations, integrations, config.
3. Write the pillars, one or more html docs each:
   - `spec`: what the app does, for whom, its main flows and rules, as the code enforces them.
   - `architecture`: modules and how they talk, runtime pieces, key decisions you can see.
   - `db`: tables or collections, columns, relations, constraints.
   - `research`: only what the repo itself records (notes, ADRs, constraints from integrations); leave the pillar empty otherwise.
4. `ui`: with a Penpot file, `omni import penpot <file.penpot> --project <id>`; it writes the pages and boards as a design doc, and the tokens, components, colors and typographies as the design system. Read its report of what could not be imported and mention it to the human. Without one, describe the existing UI in an html doc in the app-level `ui` pillar: screens, their parts, states, navigation, visual language.
5. Hand off with the links and the list of what you could not tell from the code, as a thread (`omni thread create "<question>"`).

Done when spec, architecture and db each have a doc with written content, `ui` has the imported design or the html description, and the human has the links.

## Writing well

- One idea per block (a paragraph, a list item, a table row, a layer): comments pin to a block, so a fat block cannot be pointed at.
- Write the state you want, as a wish: "The cart keeps items for 30 days", not "I added expiry".
- Write long content to a file first and build the operations with `jq`; shell quoting of HTML breaks otherwise.
- A failed batch changes nothing. Read the error, fix that operation, resend.
