# The design notes

One html doc on the feature, pillar `ui`, titled "Design notes". It tells the human what the last run did, and tells the next run which board shows which situation. Boards and behaviour nodes have no link of their own in OmniState; this doc's table is the link.

Write it as one `<section>` with these parts, one fact per block (a paragraph, a list item, a table row), so a comment can point at it:

1. **`<h2>Last run</h2>`**: what was drawn or changed (boards by name), what the checks found and what is still open, each comment answered and how, and the tree nodes that left the tree while their board stays.
2. **`<h2>Motion for the code</h2>`**: the list "Beyond the prototype" of [motion.md](motion.md), with this design's values.
3. **`<h2>Assumed</h2>`**, only for a feature without a behaviour tree: each assumption in one line.
4. **`<h2>Boards and situations</h2>`**: the table below.

## The table

One row per behaviour node the design covers, in tree order. A user story's own node has a row too, naming the board where the story begins, and the root has one saying `no board: the feature as a whole`. For a feature without a tree: one row per board, with the path of the job it serves in the Node column (the plan's path for the job, then a short name for the situation: `rename/name-taken`) and nothing in the Node ID and Seen as columns.

| Column | Holds |
| --- | --- |
| Node | the node's path (`player-logs-a-round/no-connection`) |
| Node ID | the node's ID |
| Seen as | the node's fingerprint when its boards were last drawn or checked |
| Board | the board's name, or `no board` with the reason (the outcome is an email; same as another node, with that node's path) |
| Board ID | the board's ID; several rows may name one board, and a node shown on two boards has two rows |

## Fingerprints

A node's fingerprint changes when the node is edited or moved, when a question or remark is added to it, and when an answer on it changes. Compute all of them with:

```sh
omni --as <run>/<role> --feature "<feature>" behaviour list --json > <scratch>/nodes.json
omni --as <run>/<role> --feature "<feature>" behaviour questions --json > <scratch>/questions.json
jq -r --slurpfile q <scratch>/questions.json '
  ($q[0].questions | group_by(.node_id)
    | map({key: .[0].node_id, value: "q\(length).s\(map(.version) | add)"}) | from_entries) as $f
  | .nodes[] | "\(.id)\t\(if .path == "" then "." else .path end)\tv\(.version).\($f[.id] // "q0.s0")"' <scratch>/nodes.json
```

Each line is a node's ID, its path and its fingerprint (`v2.q3.s4`).

**Which fingerprint a row gets.** A row's Seen as says what the design last took into account, so:

- Only the rows of nodes a run drew or reworked change. A node the run did not work on keeps its row as it is, however its fingerprint reads now, so its change is still there for the next run.
- The row gets the fingerprint the node had when the run started, so an edit the human makes while the run is going is seen next time.
- A node the run itself put a question on (an assumption) gets the fingerprint as it is at the end of the run, so the run's own question does not count as a change.

## What a rerun reads from it

- **Changed:** a row whose Seen as differs from the node's fingerprint now. Its boards are reworked.
- **New:** a node of the tree with no row. It gets a board, or a `no board` row.
- **Gone:** a row whose node is no longer in the tree. Its board stays; the run names it under Last run for the human to decide.

A change in the tree's rules does not change a fingerprint. The human points at the board with a comment instead.

## Writing it

Create the doc once (`omni --as <run>/<role> doc create --feature "<feature>" --kind html --pillar ui --title "Design notes"`) and insert the section:

```sh
jq -Rs '[{op:"insert",parent:null,position:{after:null},content:.}]' <scratch>/notes.html \
  | omni --as <run>/<role> doc edit <doc> --body-file -
```

On a later run, read it (`omni --as <run>/<role> doc read <doc>` prints every block with its `data-omni-id` and `data-rev`) and change only what changed: `replace` the Last run part, `set` the cells of the rows of nodes this run worked on (`{"op":"set","node":"<td id>","rev":<rev>,"properties":{"html":"v2.q3.s4"}}`), `insert` rows for new nodes under the table's `<tbody>`. Leave `data-omni-id` and `data-rev` out of what you write.
