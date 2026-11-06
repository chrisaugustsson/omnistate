# Operations

Every change to a doc is a batch of operations. One engine applies them for every
doc type (`crates/omni-core/src/ops.rs`, `EditDoc::apply`), so edits, conflicts
and history behave the same everywhere. The engine edits HTML docs (see
`docs/formats/html-docs.md`), and design files use the same operations on their
own nodes (see Designs below).

A batch is a JSON array of operations. An operation is an object with an `op`
(`insert`, `replace`, `set`, `move` or `delete`) and the fields below. A missing
field, an unknown field, a node ID that is not 8 lowercase base32 characters and
a negative revision are all errors when the batch is read.

## Where a batch goes

A batch applies to the live doc: `POST .../docs/{doc_id}/ops` with the body
`{"ops": [...]}`, or `omni doc edit <doc>`. It is applied to
the doc's head all or nothing, under a lock on the doc, so batches of agents that
write one doc at once are applied one after the other, each to the head the one
before made. The answer is 200 `{"results": [...]}`, one result per operation
with the `node`, its new `revision`, and the IDs it `created` and `removed`; the
next read of the doc has the change.

Nothing about a head is held in memory. The head of an HTML doc is a chunk with
`data-omni-id` and `data-rev` on every block-level element, and `data-rev` is where a node's
revision is kept between batches, so an API that is started afresh checks
revisions against what was written. Every applied batch adds one row to the
doc's change log (`doc_changes`: the agent, the operations as sent, the head
before and after) and sends `doc.changed` with the doc ID and the new head. A
refused batch (a conflict, an invalid operation, a body that is not a batch)
changes, logs and sends nothing. A batch of no operations changes nothing either.

## Nodes, revisions and positions

A node is addressed by its ID. In an HTML doc a node is a block-level element at
any depth: a `<td>` in a table in a `<details>` in a `<div>` is a node, and its
parent is the nearest node above it (the `<tr>`; a `<tbody>` or a `<span>`
between belongs to the content). Every node has a revision, a number that starts at
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

Adds a new node as a child of `parent`, at `position`. `parent` is any node of the
doc, at any depth.

```json
{
  "op": "insert",
  "parent": "b0000001",
  "position": {"after": "b0000003"},
  "content": "<p>A new paragraph</p>"
}
```

`content` is the HTML of exactly one block-level element (any HTML, CSS and
inline JavaScript inside it). The element and every block-level element inside
it are new nodes. An element keeps a `data-omni-id` it carries when that is a
valid node ID that no other element of the content or the doc has; otherwise
the engine assigns the ID, and the one the content carried is gone. Each new
node starts at revision 1. The result lists the new IDs in `created`, in
document order. A `data-rev` in the content is ignored.

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
be another kind of element (a `<p>` becomes an `<h3>`), as long as it is a
block-level element that reads back where it goes. The content's root keeps the
replaced node's ID, whatever `data-omni-id` it carries.

Descendants of the node are matched by ID:

- A descendant whose `data-omni-id` is in the content is kept, with that ID. Its
  revision rises by 1 only if its own content changed.
- A descendant that the content leaves out is removed, and listed in `removed` in
  document order.
- A block-level element in the content without a `data-omni-id` is new. It gets a
  new ID, revision 1, and a place in `created`.

An ID in the content that belongs to another node of the doc is not taken: the
element gets a new ID and the other node is not touched. A valid, unique ID that
belongs to no node is kept.

## set

Changes the inner HTML of a node, keeping the element itself.

```json
{
  "op": "set",
  "node": "b0000002",
  "rev": 1,
  "properties": {"html": "Orders, <code>v2</code>"}
}
```

`html` is the only property of an HTML doc: the inner HTML of the element, with
any markup. The revision rises by 1. A node inside the old content that the new
HTML leaves out is removed (listed in `removed`); one whose `data-omni-id` is in
the new HTML is kept, and new block-level elements are new nodes (`created`). An
unknown property, `id` and `style` are errors that name the property. An
element that holds nothing (`<hr>`) takes no `html`.

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
cannot go after itself, and cannot go where the HTML parser would read it
differently (see Validation).

## delete

Removes a node and all its descendants.

```json
{"op": "delete", "node": "b0000008", "rev": 1}
```

The result lists the node and its descendants in `removed`, in document order. The
parent's revision does not change. Later operations that mention a deleted node
are errors, because it is no longer in the doc. (The history of a doc is never
lost: its `doc_changes` rows keep the head from before the delete.)

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

## Designs

A design doc takes the same five operations on its nodes (pages, boards,
frames, text, images, vectors and instances), applied by
`omni_core::design::DesignDoc::apply` (`crates/omni-core/src/design/edit.rs`)
through `omni design edit`, which sends a batch to the same route as HTML docs.
A batch to a design's head replaces only the chunks of the boards it changes
and writes a new manifest; the revisions live in the manifest, so a changed
revision does not change a chunk. Instances are checked against the design
system's head.
Positions are the same, and the `parent` of a page is `null`. `content` is one
tag of the tag format with its children (`docs/formats/design-tags.md`), read
against the node it goes into, so a board goes under a page only, a page at the
top only, and every other node under a board or a frame.

- `insert` adds a subtree. A tag that carries an `id` is refused: the server
  assigns the 8-character ID of every new node, new to the project, and lists
  them in `created` in document order. Every new node is at revision 1 and the
  parent's revision does not change. A new board is an insert under a page.
- `replace` keeps the descendants whose `id` the content names, removes the ones
  it leaves out (`removed`) and creates the tags without an `id` (`created`). The
  replaced node's revision rises by 1, and a kept descendant's only when its own
  content changed.
- `set` names properties as the attributes of the node's tag, with JSON values:
  `{"content": "Hej", "fontSize": 18}`. A `null` takes an attribute away. Only
  that node's revision rises. The node's tag is written with the change and read
  again, so every rule of the tag format holds for the result. An attribute the
  tag does not take, and `id`, are errors that name it.
- `move` puts the node and its subtree under a new parent that accepts it, and
  `delete` removes it with its descendants, listed in `removed` in document order.

An ID that is in no node of the doc (an operation's `node` or `parent`, an
`id` in a replace's content, the `component` of an `<Instance>`) is a 400
`invalid_operation` whose message names it; a parent or neighbour that is not in
the doc is named before a stale `rev` is.

The body may add `"board": "<board ID>"`. Every operation of the batch must then
stay inside that board: its node, and its parent for an insert or a move, are the
board or in it, or the batch is refused (400 `invalid_operation`, naming the node).
An ID that is no board of the doc is a 404 `unknown_board`.

## Conflicts

A revision mismatch is a conflict, not a failure of the content. The conflict
says which operation (`index`, counting from 0), which `node`, the revision it
`expected`, the node's `current_revision`, and the node's `current_html`: its
current state as HTML with `data-omni-id` (for a design, as tags with `rev`). The agent reads it, decides how to
merge, and sends a new batch with the current revision.

```text
conflict in operation 2: b0000004 is at revision 3, not 1
current_html: <p data-omni-id="b0000004">Changed by someone else</p>
```

Every other refusal is an error with the operation's `index` and a message that
names the ID, tag or property that is wrong, for example "`<tr>` cannot go inside
`<section>`" or "the node `b0000fff` is not in the doc".

## Validation

An HTML doc holds any HTML, so content is not cleaned; it is checked for one
thing: that the doc still reads back. After an insert, a replace, a set or a move
the engine prints the changed element inside the elements above it and reads
that HTML again with the HTML parser. If the parser would close, move or drop
something (a `<div>` or a `<p>` in a `<p>`, a `<tr>` or a `<td>` in a
`<section>`, a `<li>` in a `<li>`), the operation is refused with the tag named,
and the doc stays as it was. Content must hold one block-level element: text or
a second element beside it, an inline element such as `<span>` and an empty
string are errors. After every batch the doc prints HTML (`to_html_with_revisions`)
that `EditDoc::from_html` reads back as the same doc.

A design doc checks the destinations of interactions when they are written (an
insert, a replace, or a set that writes `interactions`), at any depth of a
`conditional`: a `navigate`, `openOverlay` or `swapOverlay` `dest` is a board of
the doc, a `scrollTo` `dest` a layer below the top-level board that holds the
trigger, and a `changeTo` sits on an instance with a `dest` in the variant set
of the component it places. A `dest` that is in no node of the doc is refused as
"not in the doc". Deleting a board leaves the interactions that point at it as
they were, and a set that does not write `interactions` is accepted over them.

A design doc also checks the token references of every node an insert, a
replace or a set writes, with the nodes below it, against the project's design system: the token must exist ("`$state.nope`
is not a token of the design system"), be of a type the field takes ("`$color.ink`
is a color token; `content` takes a string, number or boolean token"), and have the field in its
`scopes` when it lists any ("`$state.count` is not scoped to `opacity` (scopes:
content)"). A project whose design system has no head refuses any reference with
"the project has no design system: import tokens first". The first reference that
fails is reported, as `invalid_operation` with the index of the operation. A node
that already holds a reference to a missing token is refused when a write
touches it, whatever the write changes.

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
