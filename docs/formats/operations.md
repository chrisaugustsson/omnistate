# Operations

Every change to a doc is a batch of operations. One engine applies them for every
doc type (`crates/omni-core/src/ops.rs`, `EditDoc::apply`), so edits, conflicts
and history behave the same everywhere. Today the engine edits HTML docs (see
`docs/formats/html-docs.md`); design files use the same operations on their own
nodes.

A batch is a JSON array of operations. An operation is an object with an `op`
(`insert`, `replace`, `set`, `move` or `delete`) and the fields below. A missing
field, an unknown field, a node ID that is not 8 lowercase base32 characters and
a negative revision are all errors when the batch is read.

## Nodes, revisions and positions

A node is addressed by its ID. Every node has a revision, a number that starts at
1 when the node is created and rises by 1 each time an operation changes that
node, and no other node.

An operation that changes an existing node lists that node's revision in `rev`.
If the node is at another revision, someone changed it since the agent read it:
the operation is a conflict, and the engine changes nothing (see Conflicts).

A position says where a node goes among the children of a parent:

- `{"after": "k3f9x2qa"}` puts it right after that child of the parent.
- `{"after": null}` puts it first.

A parent is a node ID, or `null` for the top level of the doc. Both `parent` and
`after` must be present, even when they are `null`.

## insert

Adds a new block as a child of `parent`, at `position`.

```json
{
  "op": "insert",
  "parent": "b0000001",
  "position": {"after": "b0000003"},
  "content": "<p>A new paragraph</p>"
}
```

`content` is the HTML of exactly one block, in the doc type's format. The block
and every block inside it are new: the engine assigns their IDs, so the content
must not carry `data-node-id`. Each new node starts at revision 1. The result
lists the new IDs in `created`, in document order.

An insert neither checks nor changes the revision of the parent, so it needs no
`rev`. Children are ordered by fractional indexes (see Order), so agents that
insert into the same parent at the same time never conflict: both inserts apply.
When two inserts go after the same child, the one applied later lands closer to
that child.

Inserting at the top level and at the first position:

```json
{"op": "insert", "parent": null, "position": {"after": null}, "content": "<section><h2>Overview</h2></section>"}
```

## replace

Swaps a node's content for new content, in one step.

```json
{
  "op": "replace",
  "node": "b0000003",
  "rev": 1,
  "content": "<p>Rewritten with <em>emphasis</em></p>"
}
```

The node keeps its ID and its place, and its revision rises by 1. The content may
be another kind of block, as long as the parent accepts it. The content's root may
carry the node's `data-node-id` or none; any other ID is an error.

Descendants of the node are matched by ID:

- A descendant whose `data-node-id` is in the content is kept, with that ID. Its
  revision rises by 1 only if its own content changed.
- A descendant that the content leaves out is removed, and listed in `removed` in
  document order.
- A block in the content without a `data-node-id` is new. It gets a new ID,
  revision 1, and a place in `created`.

An ID in the content that belongs to another node of the doc, or to no node at
all, is an error that names the ID.

## set

Changes properties of a block, keeping everything else.

```json
{
  "op": "set",
  "node": "b0000002",
  "rev": 1,
  "properties": {"level": 2, "html": "Orders, <code>v2</code>"}
}
```

Several properties in one `set` are one change: the revision rises by 1. The
properties for HTML docs are:

| Property | Value | Block |
| --- | --- | --- |
| `level` | 1 to 6 | heading |
| `kind` | `note`, `warning` or `decision` | callout |
| `ordered` | true or false | list |
| `header` | true or false | table cell |
| `alt` | text, or null to remove it | image |
| `caption` | text | figure |
| `html` | the inner HTML of the block, inline markup only | heading, paragraph, quote, list item, table cell, callout, requirement |

A property that does not fit the block, an unknown property and `id` are errors
that name the property. `html` goes through the same parsing and cleaning as any
other content.

## move

Moves a node, with everything inside it, to a new parent and position.

```json
{
  "op": "move",
  "node": "b0000004",
  "rev": 1,
  "parent": "b0000005",
  "position": {"after": null}
}
```

The node keeps its ID and its descendants keep theirs. Only the moved node's
revision rises; the old and the new parent are not touched. A move within one
parent reorders its children. A node cannot move into itself or a descendant,
cannot go after itself, and cannot go into a parent that does not accept it (a
paragraph into a list, a list item into a section).

## delete

Removes a node and all its descendants.

```json
{"op": "delete", "node": "b0000008", "rev": 1}
```

The result lists the node and its descendants in `removed`, in document order. The
parent's revision does not change. Later operations that mention a deleted node
are errors, because it is no longer in the doc. (The history of a project is
never lost: the version the node was last in keeps it.)

## Batches

A batch is all or nothing. Operations apply one after the other, so a later
operation sees what the earlier ones did: it can insert into a node that an
earlier operation moved, and its `rev` is checked against the doc as it stands at
that point. If any operation fails, whether it is a conflict or invalid, the doc
is exactly as it was before the batch: no operation of the batch applies, and no
revision or order changes.

```json
[
  {"op": "set", "node": "b0000003", "rev": 1, "properties": {"html": "Changed"}},
  {"op": "insert", "parent": "b0000001", "position": {"after": "b0000003"}, "content": "<p>New</p>"},
  {"op": "delete", "node": "b0000008", "rev": 1}
]
```

A successful batch returns one result per operation, in order:

```text
[
  {"node": "b0000003", "revision": 2, "created": [], "removed": []},
  {"node": "c0000001", "revision": 1, "created": ["c0000001"], "removed": []},
  {"node": "b0000008", "revision": 1, "created": [], "removed": ["b0000008"]}
]
```

A deleted node's result keeps the revision it had.

## Conflicts

A revision mismatch is a conflict, not a failure of the content. The conflict
says which operation (`index`, counting from 0), which `node`, the revision it
`expected`, the node's `current_revision`, and the node's `current_html`: its
current state as HTML with `data-node-id`. The agent reads it, decides how to
merge, and sends a new batch with the current revision.

```text
conflict in operation 2: b0000004 is at revision 3, not 1
current_html: <p data-node-id="b0000004">Changed by someone else</p>
```

Every other refusal is an error with the operation's `index` and a message that
names the ID, tag or property that is wrong, for example "line 1: `<span>` is not
allowed inside `<p>`" or "the node `b0000fff` is not in the doc".

## Validation and cleaning

Every operation runs the doc type's rules on its content, not only when the doc
is saved. For HTML docs these are the rules of `docs/formats/html-docs.md`: only
the block set, links with http, https or mailto, images that are attachments, and
no `style`. Content is also checked in the context of its parent: a list holds
list items, a table rows and a row cells, and text blocks hold no blocks. Dirty
inline SVG is cleaned, not rejected. Content that breaks the rules is refused
with the problem named, and the doc stays as it was. After every batch the doc is
a valid doc: it prints HTML that parses back to the same doc.

## Order

Children are kept in order by a sort key, a fractional index: a string compared as
text, such as `0i` or `0i9`. The key of a new node falls strictly between the keys
of the two children it goes between, so an insert or a move never changes the key
of any other child, and the keys of the other children never need renumbering.
There is always room for another key before the first child, after the last one and
between two neighbours, however many inserts came before. Concurrent inserts into
the same parent therefore never conflict.

## The engine in WebAssembly

`omni-core` compiles natively and to WebAssembly. `tests/ops.rs` runs the same
tests in both: `cargo test -p omni-core --test ops` natively, and
`wasm-pack test --node` for WebAssembly (`make test` runs both).
