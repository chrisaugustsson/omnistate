---
name: omni-how
description: Work out how a feature is built and write it into OmniState, as a sketch of its operations, its tables in full and an interactive map of how they fit together, with every real choice marked for the human. Use when a feature's behaviour and UI are settled and the human wants its how before tasks are cut, or says 'omni-how'.
---

Write a feature's **how** into OmniState: a shared outline of how the feature is built, so that agents building different tasks pull in the same direction. It is three docs in the feature, found later by their titles:

| Title | Pillar | Holds |
|---|---|---|
| `Map` | `architecture` | An interactive map of the feature and the existing parts it touches. The human reads it first. |
| `Operations` | `architecture` | The operations as a sketch: a name and one line each. |
| `Tables` | `db` | The tables in full: columns, types and keys. |

The how is a **starting point**. A task that finds it wrong builds what is right and writes the change back ([After this step](#after-this-step)), so write each doc at the depth its section asks for and stop there.

You write it alone. Where you meet a real **choice** you pick, give the reason and mark it. The human reads the result and answers with comments, so everything you would ask them becomes a marked choice or a line in the report.

## Start

1. Read the feature from `OMNI_FEATURE`; without it, ask the human which feature, and pass `--feature "<name>"` on every `omni behaviour` and `omni design` command too. Follow the Start steps of the `omni` skill, with a run name such as `how-1` and the role `how`.
2. Find the Spec doc: `omni doc list --feature "<name>" --pillar spec`. Without one, stop: tell the human the how builds on the Spec doc, which `omni-grill` writes.
3. Read whatever else exists. Each is optional:
   - **Project spec**: `omni doc list --global --pillar spec`, title `Project spec`. Its `Rules` bind the how; its `Stack` is the technology.
   - **Behaviour tree**: `omni behaviour list` for the nodes with their summaries, then `omni behaviour read <node>` for each node that has children: it prints the branch above it in full (the `omni` skill's `behaviour.md`). Read a leaf only when its summary leaves open what the product must do. A user story is **in scope** unless its status is `out-of-scope`.
   - **Design**: `omni doc list --feature "<name>" --pillar ui`. Per design doc, `omni design flows --doc <id>` lists its boards and flows, and `omni design read --doc <id> --board <id>` prints a board.
   - **App-level docs**: the `architecture` and `db` pillars of `omni doc list --global`.
   - **Earlier how**: docs titled `Map`, `Operations` or `Tables` in the feature.
   - **Code**: whether the current folder holds the product's source.
4. Tell the human what you found, one line per source, and carry on:

   > Spec: 9 decisions. Project spec: React, Rust, Postgres; hosting open. Tree: 3 stories, 27 nodes, 1 question open. Design: 6 boards. Code: yes. Earlier how: none, so this is a first run.

What a missing source changes, which the same message says:

- **No Project spec**: the stack and the conventions come from the code.
- **No tree**: the flows come from the Spec's `Goals`, one per thing the user can do.
- **An open question or an open node in the tree**: outline what is settled. What hangs on the open part waits, and the report names it with its node. A rule of the tree binds also while a question about it is open: outline by the rule and name the question as waiting.
- **No design**: the screens come from the tree's situations, or from the Spec without a tree, one per place the user stands, and each card's `about` says the screen is not designed yet.
- **No code**: see [Without code](#without-code).
- **An earlier how**: this is a rerun, and [Run again](#run-again) takes the place of the sections from here to it.

## Read the code

Delegate the reading to read-only sub-agents, in parallel, one question each (without sub-agents, read it yourself). Ask for facts with file paths:

- What exists that the feature touches: the screens or commands, operations, tables and outside services that the user stories pass through or the Spec names, and which of them call which. Give the agent the story sentences and the Spec's goals. Per part: its name as the code has it, one line on what it does, its file.
- Every table the feature reads or writes, as the schema or the migrations state it: all columns with type, null and default, and the keys and indexes.
- The conventions a new operation and a new table follow here: how an operation is exposed, named and reports a failure; how tables and columns are named; the type of a key, timestamps, who owns a row, how a deletion is stored; where migrations live.

Done when every existing part you are about to name traces to a file a sub-agent named.

### Without code

A project with no code has nothing to read and no conventions to follow. The `Stack` part of the Project spec is the technology. Each `Stack` line that says `open` and that this feature's how depends on, and each convention from the last bullet above that the feature needs, is a **choice**: pick it, give the reason and mark it. With neither code nor Project spec, the stack itself is the first choice. Every part is new, so nothing on the map is faint.

## The outline

An **operation** is one thing the product does on request: an endpoint, a job, the handler of a message. A command the user types is an entry point, like a screen: it stands in `Called from`, and what it calls is the operation.

Find the operations by walking each user story in scope through the tree, situation by situation: wherever the product must fetch, store, change or reach an outside service, there is an operation. Then walk each screen of the design: what it shows needs an operation that reads, and each of its actions one that writes. Merge the ones that do the same thing, and use an existing operation where one already does the job.

A table holds what must outlast a request: the things the Spec and the tree say the product remembers, bound by the Project spec's `Rules` and the tree's rules.

The code's conventions are settled: follow them in every name, type and key.

Write both docs as html docs (the `omni` skill's `html-docs.md`), one row or line per fact, so a comment can point at it. **New** and **changed** are measured against the app before this feature, and stay so once the parts are built. A part of the feature that the code already holds is listed as new or changed like the rest, with its `Shape` written from the code and what is still missing named. The examples are from a booking app on Postgres.

### `Operations`

A doc in the pillar `architecture` titled `Operations` (`omni doc create --feature "<name>" --kind html --pillar architecture --title "Operations"`), holding:

- One opening line: the feature, and that this is a starting point.
- `New and changed`: a table with one row per operation.
  - `Operation`: its name in plain words, verb first.
  - `What it does`: one line.
  - `Called from`: the screens or commands that call it.
  - `Uses`: the tables it reads or writes and the outside services it reaches.
  - `Shape`: `Not built yet.` The task that builds the operation decides how it is called, what goes in, what comes back and how it fails, and writes that here when it lands.
- `Builds on`: a table of the existing operations, screens and outside services the feature uses as they are, with `Part`, `What it is`, `Connected to` (what calls it and what it calls, among the parts in the two docs) and `Where` (its file).

```html
<tr><td>Book a time</td><td>Stores a booking when the time is still free, and confirms it by email.</td><td>Pick a time</td><td>writes bookings, customers; Email provider</td><td>Not built yet.</td></tr>
```

A feature without operations gets the doc with one line that says so.

### `Tables`

A doc in the pillar `db` titled `Tables`. Tables are written in full because they are where tasks collide. Per new or changed table:

- An `h2` with the table's name.
- A line: `New.` or `Changed: <what>.`, then what one row is.
- A table of every column: `Column`, `Type` (in the database's own types), `Null`, `Default`, `Note`. A changed table is written whole, as it will be, with `New.` or `Changed.` in the note of each column that differs.
- A list of its keys and indexes, one line each: primary key, unique, foreign key with what a delete does, index.

```html
<h2>bookings</h2>
<p>New. One row per booked time.</p>
<table>
<tr><th>Column</th><th>Type</th><th>Null</th><th>Default</th><th>Note</th></tr>
<tr><td>id</td><td>uuid</td><td>no</td><td>gen_random_uuid()</td><td></td></tr>
<tr><td>workshop_id</td><td>uuid</td><td>no</td><td></td><td></td></tr>
<tr><td>starts_at</td><td>timestamptz</td><td>no</td><td></td><td></td></tr>
<tr><td>status</td><td>text</td><td>no</td><td>'booked'</td><td>booked or cancelled</td></tr>
</table>
<ul>
<li>Primary key: id.</li>
<li>Foreign key: workshop_id to workshops.id, on delete restrict.</li>
<li>Index: workshop_id, starts_at.</li>
</ul>
```

End with `Used as they are`: a table of the existing tables the feature reads or writes without changing them, with `Table`, `Columns the feature uses`, `Used by` (the operations) and `Where` (the file that defines it).

A store that is not a relational database gets the same per collection or file: fields, types, keys. A feature without tables gets the doc with one line that says so.

### Choices

A choice is **real** when two sound builds would differ and the difference is costly to undo, or reaches another task or the user. What the Spec, the Project spec, the tree or the code's conventions decide is settled: follow it.

For each real choice, pick, and write one block:

```html
<aside data-callout="decision"><strong>Choice C1, Book a time.</strong> The free check and the insert run in one transaction, so two car owners cannot take the same time. Instead of: a unique index on workshop and start time, which cannot keep bookings of different lengths apart.</aside>
```

- The block holds the ID, what it concerns, the pick, the reason and what you passed over. The viewer draws it as a green box, and one block per choice lets a comment land on that choice.
- Where: a choice about an operation goes right after the `New and changed` table; one about a table, after that table's list of keys, or under the opening line of `Tables` when the table has no section; one about the whole feature (the stack, a convention), under the opening line of `Operations`.
- IDs run `C1`, `C2`, ... across both docs in the order written and are never renumbered.

## The map

When both docs are written, draw the map: read `map.md` in this skill's folder and follow it.

## Run again

With an earlier how, the run brings the docs in line with what is true now and draws a fresh map. A doc of the three that is missing is written as on a first run.

1. Comments first: run the `omni` skill's main loop on the how docs. A comment on a choice is the human's answer to it: change the pick and everything that follows from it, also where the old pick is already built, and name that code in the report. A comment that opens a new choice gets a new choice block, and your reply names its ID.
2. Read the code again ([Read the code](#read-the-code)) and hold the two docs against the code in the current checkout, the Spec, the tree and the design as they now are:
   - The code and a doc disagree about a part that is built: the code is right, so correct the doc.
   - An operation exists in the code and its `Shape` still says `Not built yet.`: write its shape from the code.
   - The Spec, the tree or the design changed: rework what follows from the change ([The outline](#the-outline)).
3. Edit only the nodes that change (`replace`, `set`, `insert`), so the human's comments stay on nodes that still exist. A choice keeps its ID; one that no longer applies says so in its block.
4. Draw the map again from the docs and the code (`map.md`). Tasks leave the map alone, so the one in OmniState may be stale.

## Done

Done when all of these hold:

- every user story in scope (without a tree: every thing the `Goals` say the user can do) is carried by the rows of `Operations`, or the doc says the feature has none;
- every new or changed table in `Tables` has each column typed and its keys listed;
- every pick you made under [Choices](#choices) is a block with an ID;
- the `Map` doc plays one flow per user story in scope, or the report says why no map was drawn;
- the human has the report.

The report, in the chat:

- The links to the how docs, `Map` first (the `omni` skill's Recipes say how a link is built).
- **Choices**: each ID with its pick in one line and the doc it is in.
- **Waiting**: what hangs on an open part of the tree, with the node.
- On a rerun: what changed in the docs, and whether the code, a comment or a changed Spec, tree or design caused it.
- What comes next: "Read the map, then the two docs, and comment on any block. Run omni-how again after you have commented, or for a fresh map once tasks have landed." Cutting the tasks is the tasks step's job, on the human's go.

Leave out a part that has nothing in it.

## After this step

The two outline docs stay true through the tasks that build the feature. `write-back.md` in this skill's folder is the rule a building task follows when it builds an operation or changes the how. It stands on its own, for the tasks and deliver steps to hand to their agents.
