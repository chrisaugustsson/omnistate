# HTML docs

The format of specs, architecture docs, database docs and research notes: free-form
HTML. A doc holds any markup, any CSS and inline JavaScript. `omni-core` reads it
with an HTML parser the way a browser does (`crates/omni-core/src/html_doc.rs`),
keeps what was written, and prints it back as HTML that reads as the same doc.

Agents write this HTML. Nothing is rejected for its tags: `<style>`, `<script>`,
`<svg>`, tables, `<details>`, custom classes, `data-*` attributes and inline event
handlers all stay as written. Text is escaped when printed (`&lt;`, `&gt;`, `&amp;`),
so text can never turn into markup. The only thing an operation refuses is HTML
that would not read back as the same doc (see Operations).

## Nodes and `data-omni-id`

Comments, operations and history address parts of a doc, so every block-level
element is a node with a stable ID in a `data-omni-id` attribute: 8 lowercase base32
characters (0-9 and a-z without i, l, o and u), such as `k3f9x2qa`.

The block-level elements are the HTML spec's flow content that is not phrasing
content: `address`, `article`, `aside`, `blockquote`, `details`, `dialog`, `div`,
`dl`, `fieldset`, `figure`, `footer`, `form`, `h1` to `h6`, `header`, `hgroup`,
`hr`, `main`, `menu`, `nav`, `ol`, `p`, `pre`, `search`, `section`, `table` and
`ul`, plus `li`, `tr`, `td`, `th`, `figure` and `svg`. Nothing else gets an ID:
inline elements (`<span>`, `<a>`, `<strong>`, `<code>`, `<img>`, `<br>`), the
elements inside an `<svg>`, `<thead>`, `<tbody>`, `<summary>`, `<figcaption>`,
`<style>` and `<script>` are part of the content of the node that holds them. A
nested `<svg>` is part of the outer one.

A node's parent is the nearest node above it, at any depth: a `<td>` in a table in
a `<details>` in a `<div>` is a node, and its parent is its `<tr>`. Operations
(`docs/formats/operations.md`) insert, replace, set, move and delete any node.

An agent may write a `data-omni-id` itself. It is kept when it is valid and unique:
no other element of the content or of the doc has it. An invalid ID (uppercase,
the wrong length, an `i`, `l`, `o` or `u`) and an ID used twice is replaced by a new
one, and the old value is gone. A block-level element without one gets an ID when
the content is written. A doc under edit, as the API prints it, has a
`data-omni-id` on every block-level element.

`data-rev` is the revision of a node. The API prints it next to the ID when an agent
reads a doc to edit it (`omni doc read`), and reads it back; an agent does not
write it. Both `data-omni-id` and `data-rev` are reserved: on an element that is not
block-level they are dropped.

## Printing

A doc is stored as the printed HTML of its tree. Printing keeps the text of
`<style>` and `<script>` exactly, writes attributes in the order given with double
quotes, and gives void elements (`<br>`, `<img>`, `<hr>`) no end tag. Reading what
was printed gives the same tree, and printing that gives the same bytes.

## The viewer

The doc viewer shows a doc in a frame, `<iframe sandbox="allow-scripts">`, without
`allow-same-origin`. The frame has no origin of its own: the doc's scripts run, but
they cannot read the page that holds the frame, its cookies or its storage, and
their requests carry no session. The frame document has a content security policy
that blocks every request except images that are workspace attachments
(`attachment:<id>` images are shown from `/api/v1/w/<workspace id>/attachments/`):
a doc's `fetch('https://example.com')` fails with a policy violation, and so do
images, fonts, stylesheets and frames from anywhere else. Inline scripts and
handlers are allowed; scripts from the network are not.

The doc and the page talk only through `postMessage`: the frame tells the page
which node a reviewer clicked, and the page tells the frame what to show, which
nodes carry comments and which node is selected.

A doc without styles looks readable because the frame has OmniState's default
stylesheet (`web/src/viewer/doc.css`): a sans-serif body, roomy lines, a limited
line length, and table cells with padding and borders. The default stylesheet is in
a cascade layer, so a doc's own `<style>` overrides it, even with plain element
selectors such as `h1 { font-size: 11px }` or `td { padding: 0 }`. An agent may
restyle a doc freely.

## Operations

An operation's content is one block-level element. An insert, a replace, a set or a
move is refused when the HTML parser would read the result differently: a `<div>`
or a `<p>` inside a `<p>`, a `<tr>` or a `<td>` outside a table, a `<li>` inside a
`<li>`. The error names the tag, and the doc stays as it was.

## Example

A rich doc: a style, an inline script, an SVG, a table, a `<details>` and a custom
class, with a `data-omni-id` on every block-level element.

```html
<section data-omni-id="a0000001" class="spec">
<style>.card { border: 1px solid #ccc; padding: 1em; } .card h2 { color: #336699; }</style>
<h1 data-omni-id="a0000002">Order service</h1>
<p data-omni-id="a0000003">Plain, <strong>strong</strong>, <em>emphasis</em>, <code>code()</code> and <a href="https://example.com/docs">a link</a>.<br>After the line break.</p>
<div data-omni-id="a0000004" class="card" onclick="toggle(this)">
<h2 data-omni-id="a0000005">Fields</h2>
<table data-omni-id="a0000006">
<thead><tr data-omni-id="a0000007"><th data-omni-id="a0000008">Field</th><th data-omni-id="a0000009">Type</th></tr></thead>
<tbody><tr data-omni-id="a0000010"><td data-omni-id="a0000011">id</td><td data-omni-id="a0000012">uuid</td></tr></tbody>
</table>
</div>
<details data-omni-id="a0000013" open>
<summary>Open questions</summary>
<ul data-omni-id="a0000014"><li data-omni-id="a0000015">Do orders expire?</li><li data-omni-id="a0000016">Who can cancel one?</li></ul>
</details>
<figure data-omni-id="a0000017"><svg data-omni-id="a0000018" viewBox="0 0 100 50"><rect x="1" y="1" width="50" height="20" fill="#336699"/><circle cx="70" cy="25" r="10"/></svg><figcaption>A box and a circle</figcaption></figure>
<pre data-omni-id="a0000019"><code>fn main() {
    println!("hi");
}</code></pre>
<img src="attachment:0192f1c4-5b6a-7d8e-9f01-23456789abcd" alt="A screenshot">
<script>function toggle(card) { card.classList.toggle("open"); }</script>
</section>
```

A short doc with no styles of its own, which the default stylesheet makes readable:

```html
<h1 data-omni-id="b0000001">Checkout</h1>
<p data-omni-id="b0000002">Orders ship in <strong>one</strong> day.</p>
<blockquote data-omni-id="b0000003">Measure twice, cut once.</blockquote>
<aside data-omni-id="b0000004" data-callout="decision">Orders are never deleted.</aside>
<hr data-omni-id="b0000005">
<ol data-omni-id="b0000006"><li data-omni-id="b0000007">Step one</li></ol>
```
