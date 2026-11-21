---
name: omni-stories
description: Split a track into small tasks on its board in OmniState, the normal path of each user story first and its side situations after it, with criteria taken from the behaviour tree and every tree node covered by a task. Use when a track's behaviour, UI and how are settled and the human wants its tasks cut, when the tree changed after tasks were cut, or says 'omni-stories'.
---

Split a track into **tasks** on its board, each a slice an agent can build and a reviewer can check on its own. A task is a story record: the commands (`omni story ...`) and the keys (`US-<n>`) still say story. A **user story** is something else: a node of the behaviour tree, one role doing one flow.

The split runs hands off. It saves without asking, and what only the human can decide becomes a question on a tree node ([Gaps](#gaps)). It ends with the tasks on the board and a report; the human starts the build.

## Start

1. Read the track from `OMNI_TRACK`; without it, ask the human which track (`omni track list`), and pass `--track "<name>"` on every `omni story`, `omni behaviour` and `omni design` command. Follow the Start steps of the `omni` skill, with a run name such as `tasks-1` and the role `tasks`.
2. Find the Spec doc: `omni doc list --track "<name>" --pillar spec`. Without one, stop: tell the human the split builds on the Spec doc, which `omni-grill` writes.
3. Read whatever else exists. Each is optional:
   - **Project spec**: `omni doc list --global --pillar spec`, title `Project spec`. Its `Rules` bind every task; its `Stack` is the technology.
   - **Behaviour tree**: `omni behaviour list --json` gives every node with its `id`, `path`, `level`, `status`, `version`, `open_questions` and `needs_pass` (the model is in the `omni` skill's `behaviour.md`). Then `omni behaviour read <leaf>` for each leaf: it prints the branch above the leaf in full, with the rules that reach it.
   - **Design**: `omni doc list --track "<name>" --pillar ui`. Per design doc, `omni design flows --doc <id>` lists its boards, and `omni design read --doc <id> --board <id>` prints one.
   - **How**: the track's docs titled `Operations` (pillar `architecture`) and `Tables` (pillar `db`), which `omni-how` writes. They are a starting point: the task that builds a part may change it.
   - **Code**: whether the folder holds the product's source. When it does, read the closest existing code, tests and routes for what each task will change.
   - **Tasks**: `omni story list --json` gives the tasks the track already has, each with its `text`, `status` and `criteria`.
4. Pick the path:
   - A task exists whose status is not `cancelled`: this is a second run. Read [Run again](#run-again) before you cut.
   - The folder holds no source: [Without code](#without-code) comes first.
   - The track has no tree: [Without a tree](#without-a-tree) takes the place of the tree in what follows.

## What is cut

The tasks cover the tree's nodes below the root, apart from a node that is `out_of_scope` or lies under one. Of those, a node is **settled** when its status is `done`, its `open_questions` is 0 and `needs_pass` is false. A settled node gets covered by a task. Any other node **waits**: no task covers it in this run, and the report names it.

A user story whose normal path holds a node that waits, waits whole, since its other tasks build on that path.

## Cutting

A task is a vertical slice through every layer it needs (schema, server, command line, web), and the app works when it has landed.

- **The normal path first.** Per user story, the first task is its normal path: the story node and the branch below it where everything goes as the story sentence says, down to the outcome the user sees. Something works end to end when it has landed.
- **Side situations after it.** Every other branch of the user story is a task of its own, or one task together with the few that belong with it (the same screen, the same check). Each names the normal-path task in `Blocked by`.
- **Size.** At most 4 criteria per task, not counting the standard lines (below). A normal path that needs more is cut along the path: the first task reaches the first outcome the user sees, the next carries on from there.
- **Criteria come from the nodes.** A criterion states what a covered node's behaviour says the user sees, in the node's own words where they are exact: the text of a message, the state of a button, where the user ends up. A node with `same_as` takes its criterion from the node it points to; a node with `handoff` has the criterion that the user stands where that user story starts. A task covers a node when its criteria together state everything the node's behaviour says. A criterion never names a test: what the tests must pin goes under `Tests:`.
- **Standard lines**, added where they apply:
  - `Typecheck/lint passes`, on every task.
  - `Verify in browser using dev-browser skill (<what the page shows>)`, on a task that changes a screen.
  - `` `Operations` and `Tables` say what the landed code does for every operation and table this task touched (`write-back.md` of the `omni-how` skill) ``, on a task that builds or changes an operation or a table, in a track that has those docs.
- **The how.** The Description names the operations and tables the task builds or changes, by the names `Operations` and `Tables` give them. How an operation is called, what it returns and how it fails is the builder's to decide. A table is built whole, as `Tables` states it, by the first task that needs it.
- **Interfaces** name what the task adds that later tasks use (operations and tables, by those same names) and the existing code it uses, by its names in the code.
- **Dependencies.** A task that uses an operation, a table or code another task adds names that task in `Blocked by`. With no such dependency it says `Blocked by: none`. Order alone is no reason to block.
- **Prefactor.** A refactor that makes a later task easy is its own earlier task, Topic kind `prefactor`. It covers no node.
- **Files.** `change:` lists the paths the task changes, found by reading the code, as backticked paths. `read first:` lists at most 6 paths, such as the closest existing code to follow. A number that depends on the order tasks land in is written as words: "the next free migration number".
- **Tests.** One `- ` line for each test the task must have, stated as what it pins, then a last line `- copy:` with the closest existing test as a backticked path. The build's test author owns these lines.
- **Links.** `--link <doc>:<node>` for each `Operations` row and `Tables` section the task builds, changes or uses, each design board it builds, and each Spec decision that bears on it. `<doc>` and `<node>` are IDs; an optional `:<label>` follows. A tree node cannot be linked: it goes in `Covers`.

## Task file

Write each task to a file in the session's scratch directory, in this shape:

```markdown
### US-<n>: <Title>
**Description:** <what the user can do when this has landed, in 2-3 sentences>. Builds <operations and tables>.
Covers, in the track's behaviour tree (`omni behaviour read <id> --track "<name>"`):
- <node id> v<version> `<node path>`
**Topic:** <kind: feature | setup | prefactor | bugfix | chart | docs> · <1-3 domain tags>
**Blocked by:** <US-xxx, ... or none>
**Interfaces:** <what it adds for later tasks, and the existing code it uses>
**Files:** change: `<path>`, `<path>` · read first: `<path>`
Tests:
- <what a test must pin>
- copy: `<the closest existing test>`

**Acceptance Criteria:**
- [ ] <what the user sees, with the exact text, value or state>
- [ ] Typecheck/lint passes
```

- `omni story add` keeps as the task's text the Description, Topic, Interfaces and Files blocks, and a block ends at a blank line or a bold label. So the `Covers` lines stand right under the Description and the `Tests:` lines right under `**Files:**`, plain, with no blank line before them.
- `Covers` has one line per node the task covers: the node's `id` and `version` as `omni behaviour list --json` gave them, then its path. The ID and the version are how a later run finds what is covered and what has changed, so copy them exactly.
- The `US-<n>` in the heading is your own, for ordering and for `Blocked by`; the server assigns the real key. After each add, note the key it printed and use the real keys in the `Blocked by` of later tasks.

## Gaps

Before a task is saved, put each of its criteria to the **two-implementation test**: picture two builds a reasonable developer could make that behave differently for the user. When both meet the criterion's wording, the criterion decides nothing: it is a **gap**. "A taken name is handled" is met by an error under the field and by a silent `-2` on the name. "The field shows `That name is taken` and the dialog stays open" is met by one build only.

- Words like handled, supported, works, correctly, gracefully, sensible and as expected often mark a gap; the test decides.
- What the builder decides on purpose is no gap: internal structure, names, and the shape of an operation.
- Look for the answer first: an answered question on the node or above it, a rule that reaches it, the Spec's `Decisions`, the Project spec's `Rules`. One that names this exact case settles the criterion.

A gap that is left is a gap in the node, and the question goes on that node:

- **The human would notice the difference**: it changes what the user sees or can do in the node's situation, what is stored, sent or deleted, who may do it, or money; or two nodes contradict each other. `omni behaviour ask <node> "<question>" --recommend "<the answer as it should stand>"`. The node is open now and waits ([What is cut](#what-is-cut)).
- **The human would accept either build without noticing**: timing and races, the order or duplicates of input the node does not name, cosmetic detail, a case that needs two edge conditions at once, a convention the code already follows. Pick the most conventional answer, record it as your default with `omni behaviour ask <node> "<question>" --answer "<answer>"`, and write the criterion with it. The node stays settled.

Done when every criterion of every task to be saved passes the test, and every gap found is a question on its node.

## Saving

Save the tasks in dependency order, so every blocker exists before the task that names it:

```
omni story add <file> --link <doc>:<node> [--link <doc>:<node>:<label>]...
```

Pass `--priority <n>` only when the Spec says a task matters more or less than the rest (1 highest, 3 the default; 0 belongs to a task returned from review or testing). Pass `--number-from <n>` only for the project's very first task, when the human gave a number to continue from. Fix a rejected add and retry, so that each task is saved once.

When all are saved, check both:

- `omni story list --pickable` holds, of this run's tasks, exactly those with `Blocked by: none`. Correct a task of this run with `omni story edit <key> --blocked-by <keys>`.
- Coverage: collect the node IDs of the `Covers` lines of every task in `omni story list --json` whose status is not `cancelled`, and hold them against `omni behaviour list --json`. Every settled node of [What is cut](#what-is-cut) is among them.

## Without code

When the folder holds no source of the product and no task of the track is titled `Set up the project`, cut that task first, Topic kind `setup`:

- Its criteria: the folder is a git repository with the setup committed; the app starts with one command, on the stack the Project spec names; the test command runs one test, which passes; a project-level doc in the pillar `architecture`, titled `Setup`, states the folders, the libraries and the commands to start and to test.
- The builder decides those details, and each `Stack` line of the Project spec that says `open`. Say so in the Description.
- It covers no node and links the Project spec's `Stack`.
- It runs alone: every other task that would say `Blocked by: none` names it instead, so `omni story list --pickable` holds only this task.

## Without a tree

The Spec doc takes the tree's place:

- A user story is one thing the Spec's `Goals` say the user can do, and its side situations are the cases the `Decisions` settle.
- Criteria come from the goals and the decisions. Each task links the goal items and decision rows it builds (`--link <Spec doc>:<node>`) and has no `Covers` lines.
- A gap the human would notice: leave the task out, start a thread with `omni thread create "<question>" --track "<name>"`, and name it in the report. One the human would accept either way: write the criterion with your answer and record it after the add with `omni story decision <key> --run <run> --question "<question>" --choice "<answer>" --by agent`.
- Coverage: every goal item, and every decision row that changes what the product does, is linked from a task that is not cancelled.

## Run again

A second run adds tasks for what is new or changed and leaves every existing task as it is, whatever its status: no edit, no cancel, no new priority.

1. From `omni story list --json`, take every task whose status is not `cancelled` and read its `Covers` lines. Per node ID, keep the version recorded by the newest task that names it.
2. Sort the nodes of [What is cut](#what-is-cut):
   - **New**: no task names its ID. Cut it as in [Cutting](#cutting). A new side situation of a user story whose normal path has a task is blocked by that task.
   - **Changed**: its version is higher than the recorded one, or it has `same_as` and the node it points to is changed, or it is `out_of_scope` now. Read the node and hold it against the criteria of the task that covers it, since a rename, a move and a cleared mark raise the version too. When the criteria still state the node, it needs no task. Otherwise cut a new task: the Description names the task it changes, `Blocked by` names that task, the criteria state the node as it now is (for a node now out of scope: that its behaviour is gone), and `Covers` records the new version.
   - **Waiting** or unchanged: nothing.
3. A change gets a new task also when the task it changes is still in the backlog. Name both in the report: the human may cancel the old one on the board.
4. Without a tree: a goal item or decision row no task links is new; a linked one whose text the linking task's criteria no longer state is changed.

Done when every existing task has the `version` and `updated_at` it had in step 1, apart from what the build did meanwhile.

## Done

Done when all of these hold:

- every settled node is named in the `Covers` of a task that is not cancelled, and on a second run every changed node has a new task or a line in the report saying its criteria still hold;
- every node that waits is in the report, with its open question when it has one;
- no task has more than 4 criteria besides the standard lines;
- each user story's first task covers its normal path, and its other tasks are blocked by it;
- the two checks of [Saving](#saving) pass;
- the chat asked the human nothing besides which track, and the build has not been started;
- the human has the report.

The report, in the chat:

- The tasks added: key and title, which can start now and which wait for which, and the link to the track (the `omni` skill's Recipes say how a link is built).
- **Waiting**: each node that waits, with the question on it, and what the human does: "Answer on the Behaviour screen, run omni-behaviour, then omni-stories again."
- **By default**: each answer you recorded as your own, with its node.
- On a second run: per new task, the node that was new or changed, and each backlog task a new one changes.
- What comes next: "Start the build with omni-deliver when you want it."

Leave out a part that has nothing in it. Starting the build is the human's step.
