# HTML docs

The format of the PRD, architecture and database docs: a tree of typed blocks
with stable node IDs, written as a strict subset of HTML. `omni-core` reads it
(`crates/omni-core/src/html_doc.rs`, `HtmlDoc::parse`), cleans it, stores it as
JSON and prints it back as HTML (`HtmlDoc::to_html`).

Agents write this HTML. Nothing else is accepted: any other tag, any free-form
nesting and any `style` attribute is rejected with an error that names the line
and the problem, such as "line 12: `<span>` is not allowed inside `<p>`". The
doc viewer shows the printed HTML with one shared stylesheet, so a doc has no
styles of its own. Comments are not part of the format either.

## Blocks

A doc is a list of top-level blocks. White space between blocks is ignored;
text outside a text block is an error. Every block may carry a `data-node-id`.

| Block | HTML | Holds |
| --- | --- | --- |
| Section | `<section>` | blocks, including more sections |
| Heading | `<h1>` to `<h6>` | inline content |
| Paragraph | `<p>` | inline content |
| Quote | `<blockquote>` | inline content |
| Divider | `<hr>` | nothing (`<hr/>` works too) |
| List | `<ul>` or `<ol>` | `<li>` items only |
| List item | `<li>` | inline content |
| Table | `<table>` | `<tr>` rows only, with no `<tbody>` |
| Table row | `<tr>` | `<td>` and `<th>` cells only |
| Table cell | `<td>`, or `<th>` for a header cell | inline content |
| Figure | `<figure>` | one `<svg>` or one `<pre class="mermaid">`, then a `<figcaption>` |
| Callout | `<aside data-callout="note">` | inline content |
| Code block | `<pre><code>` | text only, kept exactly as written |
| Image | `<img src="attachment:<id>" alt="...">` | nothing |
| Requirement | `<p data-block="requirement">` | inline content |
| User story | `<article data-block="user-story">` | blocks |

A callout is a note, a warning or a decision: `data-callout` is `note`,
`warning` or `decision`, and nothing else. A `<p>` with any other `data-block`
and an `<article>` that is not a user story are errors.

A figure always has a caption, as text. A Mermaid figure holds the diagram as
text, so `<` and `&` are written `&lt;` and `&amp;`, and `-->` may be written
`--&gt;`. A figure without a picture or without a caption is an error.

## Inline content

Text blocks hold text with these tags and no others:

| Markup | HTML |
| --- | --- |
| Strong | `<strong>` |
| Emphasis | `<em>` |
| Code | `<code>`, text only |
| Link | `<a href="...">` |
| Line break | `<br>` (`<br/>` works too) |

They nest (`<strong>both <em>here</em></strong>`), except that a link holds no
other link. `<b>`, `<i>`, `<span>` and the rest are rejected. Text is written
with `&lt;`, `&gt;` and `&amp;` for `<`, `>` and `&`; it is unescaped when read
and escaped when printed, so text can never inject markup.

## Node IDs

A block may carry a `data-node-id`: 8 lowercase base32 characters (0-9 and a-z
without i, l, o and u), such as `k3f9x2qa`. An ID is used for one block only.
A block without one is new, and the server assigns its ID when the doc is
saved (US-022). Inline markup has no IDs: a comment lands on the text block.

## Cleaning

Parsing cleans what it keeps:

- Links must use `http`, `https` or `mailto`. Every other link (`javascript:`,
  `data:`, a relative path, a bare address) is an error.
- Images must be attachments, written `attachment:<id>`, never an address.
- Inline SVG is reduced to an allow-list. Shapes (`rect`, `circle`, `ellipse`,
  `line`, `polyline`, `polygon`, `path`), text (`text`, `tspan`, `title`,
  `desc`), `g`, `defs`, gradients, `clipPath` and `marker` stay, with their
  geometry and presentation attributes. Scripts, `style`, `foreignObject`,
  images, links, `use`, event handlers and every `href`, `xlink:href`,
  `@import` and `url()` that points outside the picture are removed. Dirty SVG
  is stripped, not rejected.

## Storage and printing

A doc is stored as its block tree in JSON, never as raw HTML: each block is an
object with its `type`, its `id` (left out while the block is new) and its
content. Printing the tree gives HTML with a `data-node-id` on every block that
has one, which parses back to the same tree, and printing that again gives the
same bytes.

## Example

A complete doc: every block type, all three callout kinds, all inline markup,
an SVG and a Mermaid figure, and a `data-node-id` on every block.

```html
<section data-node-id="a0000001">
<h1 data-node-id="a0000002">Order service</h1>
<p data-node-id="a0000003">Plain, <strong>strong</strong>, <em>emphasis</em>, <code>code()</code> and <a href="https://example.com/docs">a link</a>.<br>After the line break.</p>
<blockquote data-node-id="a0000004">Measure twice, cut once.</blockquote>
<hr data-node-id="a0000005">
<ul data-node-id="a0000006">
<li data-node-id="a0000007">First item</li>
<li data-node-id="a0000008">Second item</li>
</ul>
<ol data-node-id="a0000009">
<li data-node-id="a0000010">Step one</li>
</ol>
<table data-node-id="a0000011">
<tr data-node-id="a0000012"><th data-node-id="a0000013">Field</th><th data-node-id="a0000014">Type</th></tr>
<tr data-node-id="a0000015"><td data-node-id="a0000016">id</td><td data-node-id="a0000017">uuid</td></tr>
</table>
<figure data-node-id="a0000018"><svg viewBox="0 0 100 50"><rect x="1" y="1" width="50" height="20" fill="#336699"/><circle cx="70" cy="25" r="10"/></svg><figcaption>A box and a circle</figcaption></figure>
<figure data-node-id="a0000019"><pre class="mermaid">graph TD;
  A--&gt;B</pre><figcaption>The flow</figcaption></figure>
<aside data-node-id="a0000020" data-callout="note">A note.</aside>
<aside data-node-id="a0000021" data-callout="warning">Mind the gap.</aside>
<aside data-node-id="a0000022" data-callout="decision">Orders are never deleted.</aside>
<pre data-node-id="a0000023"><code>fn main() {
    println!("hi");
}</code></pre>
<img data-node-id="a0000024" src="attachment:0192f1c4-5b6a-7d8e-9f01-23456789abcd" alt="A screenshot">
<p data-node-id="a0000025" data-block="requirement">FR-1: The system must keep every version.</p>
<article data-node-id="a0000026" data-block="user-story">
<h3 data-node-id="a0000027">Reset a password</h3>
<p data-node-id="a0000028">As a user, I want to reset my password so that I can get back in.</p>
<ul data-node-id="a0000029"><li data-node-id="a0000030">The link works once</li></ul>
</article>
</section>
```
