# HTML docs

Specs, architecture, db and research notes are html docs: any HTML, CSS and inline JavaScript. The viewer has a readable default stylesheet; your own `<style>` overrides it. The doc runs in a sandbox with no network, so put images in with `omni attach <file>` and write `<img src="attachment:<id>">`; no external scripts, fonts or images.

## Nodes

Every block-level element (`section`, `div`, `h1`..`h6`, `p`, `ul`, `ol`, `li`, `table`, `tr`, `td`, `th`, `pre`, `blockquote`, `details`, `figure`, `svg`, `hr`, ...) is a node with a `data-omni-id`. Inline elements (`span`, `a`, `strong`, `code`, `img`) belong to the node that holds them. `omni doc read <doc>` prints the doc with `data-omni-id` and `data-rev` on every node; `--node <id>` (repeatable) prints only those nodes, each with what is inside it. The server assigns IDs; leave `data-omni-id` and `data-rev` out of what you write.

## Writing

`omni doc edit <doc> --body-file <file>` (or `--body-file -` on standard input) takes a JSON array of operations, applied all or nothing, at once:

```json
[
  {"op": "insert", "parent": null, "position": {"after": null}, "content": "<section>...</section>"},
  {"op": "replace", "node": "k3f9x2qa", "rev": 2, "content": "<p>New paragraph</p>"},
  {"op": "set", "node": "k3f9x2qa", "rev": 2, "properties": {"html": "Inner HTML, <code>kept element</code>"}},
  {"op": "move", "node": "k3f9x2qa", "rev": 2, "parent": null, "position": {"after": "m4n5p6q7"}},
  {"op": "delete", "node": "k3f9x2qa", "rev": 2}
]
```

- `insert` adds one block-level element as a child of `parent` (a node ID, or `null` for the top level), after the sibling in `position.after` (`null` is first). It needs no `rev`.
- `replace` swaps a node for new content, `set` changes its inner HTML, `move` and `delete` do what they say; each lists the node's current `rev` from the read.
- `content` is exactly one block-level element: wrap several in a `<section>`.
- A stale `rev` is a conflict: nothing changes and the error names the current revision. Read the doc again and resend.
- The result lists each node's new revision and the IDs of new nodes.

To write a whole doc, put the HTML in a file and send it as one insert:

```
jq -Rs '[{op:"insert",parent:null,position:{after:null},content:.}]' doc.html | omni doc edit <doc> --body-file - --as <run>/<role>
```

To rework a doc, read it, then `replace` or `set` only the nodes that change, so the human's comments stay on nodes that still exist.

`omni doc rename <doc> <title>` renames; `omni doc delete <doc>` hides a doc (only when the human asks); `omni doc activity <doc>` lists who wrote what.
