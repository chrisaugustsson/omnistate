//! US-109: free-form HTML docs (US-020 pinned a strict block format, which
//! this story replaces). `omni-core` reads any HTML with a general HTML
//! parser, and the operation engine (`EditDoc`) edits it.
//!
//! The tests pin this, through the engine's public API (`EditDoc::from_html`,
//! `apply`, `to_html`, `to_html_with_revisions`, `node_html`, `revision`):
//!
//! - Any HTML is accepted, including `<style>`, `<script>`, `<svg>`, tables,
//!   `<details>`, custom classes and attributes and inline event handlers.
//!   What the agent wrote is kept: the text of a `<style>` or `<script>` is
//!   kept exactly, attributes and classes are kept, text is escaped when
//!   printed, so printing what was read and reading it again gives the same
//!   doc and the same bytes.
//! - Every block-level element gets a stable ID in a `data-omni-id`
//!   attribute: the HTML spec's flow content that is not phrasing content
//!   (`address`, `article`, `aside`, `blockquote`, `details`, `div`, `dl`,
//!   `fieldset`, `figure`, `footer`, `form`, `h1`-`h6`, `header`, `hr`,
//!   `main`, `nav`, `ol`, `p`, `pre`, `section`, `table`, `ul`), plus `li`,
//!   `tr`, `td`, `th`, `figure` and `svg`. Inline (phrasing) elements, the
//!   elements inside an `<svg>`, `<thead>`/`<tbody>`, `<style>` and `<script>`
//!   get none. `data-node-id` is no longer the ID attribute.
//! - An agent-supplied `data-omni-id` is kept when it is a valid node ID (8
//!   characters of `0-9a-z` without `i`, `l`, `o` and `u`) and unique: used
//!   by no other element of the content or of the doc. Otherwise the element
//!   gets a new ID, and the old value is gone.
//! - docs/formats/html-docs.md documents the format: free-form HTML,
//!   `data-omni-id` and which elements carry it, the viewer frame's sandbox
//!   and content security policy and the default stylesheet, with ```html
//!   examples that load as docs.

use omni_core::design::NodeId;
use omni_core::ops::{EditDoc, OpResult, Operation};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU32, Ordering};

const ALPHABET: &[u8] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// Elements that are block-level and must carry a `data-omni-id`.
const BLOCK_LEVEL: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "details",
    "div",
    "dl",
    "fieldset",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "main",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "table",
    "ul",
    "li",
    "tr",
    "td",
    "th",
    "svg",
];

/// Elements that are not block-level and must carry no `data-omni-id`.
const NOT_BLOCK_LEVEL: &[&str] = &[
    "a", "abbr", "b", "br", "button", "code", "em", "i", "img", "kbd", "mark", "small", "span",
    "strong", "sub", "sup", "g", "rect", "circle", "path", "thead", "tbody", "style", "script",
];

/// A section that uses much of HTML: styles, scripts, an SVG, a table, a
/// `<details>`, custom classes and attributes, inline handlers and every
/// element of `BLOCK_LEVEL` and `NOT_BLOCK_LEVEL`.
const RICH: &str = r##"<section class="doc intro" data-theme="dark">
<style>.card > p { color: #336699; } .card::after { content: "&"; }</style>
<header><h1>Orders <small>v2</small></h1></header>
<nav><ul><li><a href="#rules">Rules</a></li></ul></nav>
<main>
<article>
<h2>Rules</h2>
<p>Plain <span class="tag">span</span>, <strong>strong</strong>, <em>em</em>, <b>b</b>, <i>i</i>, <code>code</code>, <mark>mark</mark>, <abbr title="x">abbr</abbr>, <kbd>K</kbd>, <sub>1</sub>, <sup>2</sup> and <a href="https://example.com/docs">a link</a>.<br>Next line <img src="attachment:0192f1c4-5b6a-7d8e-9f01-23456789abcd" alt="A picture"></p>
<div class="card custom" onclick="toggle(1, 2)"><p>Inside a card</p></div>
<blockquote><p>A quote</p></blockquote>
<aside data-callout="note"><p>A note</p></aside>
<details open><summary>More</summary><div><p>Hidden at first</p></div></details>
<figure><svg viewBox="0 0 10 10"><g><rect x="1" y="1" width="5" height="5" fill="#336699"/><circle cx="5" cy="5" r="2"/><path d="M0 0L10 10"/></g></svg><figcaption>A picture</figcaption></figure>
<table class="grid"><thead><tr><th>Field</th></tr></thead><tbody><tr><td>id</td></tr></tbody></table>
<dl><div><dt>Term</dt><dd>Definition</dd></div></dl>
<pre><code>fn main() {}</code></pre>
<hr>
<form><fieldset><p>Inside a field set</p></fieldset></form>
<address>Somewhere</address>
<ol><li>One</li></ol>
<h3>Three</h3><h4>Four</h4><h5>Five</h5><h6>Six</h6>
<footer><p>Footer</p></footer>
</article>
</main>
<script>function toggle(a, b) { if (a < b && b > 0) { return a; } return b; }</script>
<button type="button" onclick="toggle(2, 1)">Toggle</button>
</section>"##;

// --- Helpers ---

fn id(text: &str) -> NodeId {
    NodeId::from_str(text).unwrap_or_else(|error| panic!("{text} must be a node ID: {error}"))
}

fn is_node_id(text: &str) -> bool {
    text.len() == 8 && text.bytes().all(|byte| ALPHABET.contains(&byte))
}

/// The next ID for a new node, never used by any test or doc here.
fn new_id() -> NodeId {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    id(&format!("n{:07}", NEXT.fetch_add(1, Ordering::Relaxed)))
}

fn empty() -> EditDoc {
    EditDoc::from_html("").expect("an empty head is a doc")
}

fn apply(doc: &mut EditDoc, ops: Value) -> Vec<OpResult> {
    let parsed: Vec<Operation> = serde_json::from_value(ops.clone())
        .unwrap_or_else(|error| panic!("{ops} must be a batch: {error}"));
    doc.apply(&parsed, &mut new_id)
        .unwrap_or_else(|error| panic!("the batch must apply, but: {error}"))
        .ops
}

fn insert_top(content: &str) -> Value {
    json!({"op": "insert", "parent": null, "position": {"after": null}, "content": content})
}

/// A doc made of one top-level insert of `content`; returns the doc and the
/// insert's result.
fn doc_of(content: &str) -> (EditDoc, OpResult) {
    let mut doc = empty();
    let mut results = apply(&mut doc, json!([insert_top(content)]));
    (doc, results.remove(0))
}

/// The value of attribute `name` in the text of one start tag.
fn attribute(tag: &str, name: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let needle = format!(" {name}={quote}");
        if let Some(start) = tag.find(&needle) {
            let rest = &tag[start + needle.len()..];
            let end = rest.find(quote)?;
            return Some(rest[..end].to_string());
        }
    }
    None
}

/// One start tag of printed HTML: its lowercase name, its text and where it
/// starts.
struct Tag {
    name: String,
    text: String,
    at: usize,
}

impl Tag {
    fn omni_id(&self) -> Option<String> {
        attribute(&self.text, "data-omni-id")
    }
}

/// Every start tag of `html` in document order. The text of `<style>` and
/// `<script>` elements and comments is skipped.
fn start_tags(html: &str) -> Vec<Tag> {
    let bytes = html.as_bytes();
    let mut tags = Vec::new();
    let mut at = 0;
    while let Some(offset) = html[at..].find('<') {
        let start = at + offset;
        if html[start..].starts_with("<!--") {
            at = html[start..]
                .find("-->")
                .map_or(html.len(), |end| start + end + 3);
            continue;
        }
        let name_start = start + 1;
        let name_end = html[name_start..]
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '-')
            .map_or(html.len(), |end| name_start + end);
        if name_end == name_start || !bytes[name_start].is_ascii_alphabetic() {
            at = start + 1;
            continue;
        }
        // The end of the tag, past quoted attribute values.
        let mut end = name_end;
        let mut quote: Option<u8> = None;
        while end < bytes.len() {
            match (quote, bytes[end]) {
                (None, b'>') => break,
                (None, b'"' | b'\'') => quote = Some(bytes[end]),
                (Some(open), byte) if byte == open => quote = None,
                _ => {}
            }
            end += 1;
        }
        let name = html[name_start..name_end].to_ascii_lowercase();
        tags.push(Tag {
            name: name.clone(),
            text: html[start..end.min(html.len())].to_string(),
            at: start,
        });
        at = end + 1;
        if name == "style" || name == "script" {
            let close = format!("</{name}");
            at = html[at.min(html.len())..]
                .to_ascii_lowercase()
                .find(&close)
                .map_or(html.len(), |found| at + found);
        }
    }
    tags
}

/// The `data-omni-id` values of printed HTML, in document order.
fn printed_ids(html: &str) -> Vec<String> {
    start_tags(html).iter().filter_map(Tag::omni_id).collect()
}

/// The start tag that comes right before the first `text` in `html`.
fn tag_before<'a>(tags: &'a [Tag], html: &str, text: &str) -> &'a Tag {
    let at = html
        .find(text)
        .unwrap_or_else(|| panic!("`{text}` is in the doc: {html}"));
    tags.iter()
        .rev()
        .find(|tag| tag.at < at)
        .unwrap_or_else(|| panic!("a tag comes before `{text}`: {html}"))
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

// --- Any HTML is accepted ---

#[test]
fn any_html_is_accepted_and_kept_as_written() {
    let (doc, result) = doc_of(RICH);
    let html = doc.to_html();
    for kept in [
        "<style>.card > p { color: #336699; } .card::after { content: \"&\"; }</style>",
        "if (a < b && b > 0) { return a; } return b;",
        "class=\"doc intro\"",
        "data-theme=\"dark\"",
        "class=\"card custom\"",
        "onclick=\"toggle(1, 2)\"",
        "onclick=\"toggle(2, 1)\"",
        ">More</summary>",
        "viewBox=\"0 0 10 10\"",
        "fill=\"#336699\"",
        "<path d=\"M0 0L10 10\"",
        "<span class=\"tag\">span</span>",
        "<a href=\"https://example.com/docs\">a link</a>",
        ">A picture</figcaption>",
        ">Term</dt>",
        "<code>fn main() {}</code>",
        "<button type=\"button\"",
    ] {
        assert!(
            html.contains(kept),
            "the doc keeps `{kept}` as written:\n{html}"
        );
    }
    let names: HashSet<String> = start_tags(&html).into_iter().map(|tag| tag.name).collect();
    for name in BLOCK_LEVEL.iter().chain(NOT_BLOCK_LEVEL) {
        assert!(
            names.contains(*name),
            "the doc keeps its <{name}> elements:\n{html}"
        );
    }
    assert_eq!(
        doc.node_html(result.node)
            .expect("the inserted section prints")
            .matches("<li")
            .count(),
        2,
        "nothing is dropped"
    );
}

#[test]
fn text_is_escaped_when_printed_so_it_never_turns_into_markup() {
    let (doc, result) = doc_of(
        "<div><p>a &lt; b &amp;&amp; c &gt; d, &lt;script&gt;alert(1)&lt;/script&gt;</p><p title=\"say &quot;hi&quot; &amp; go\">Quoted</p></div>",
    );
    let html = doc.to_html();
    assert!(
        html.contains("a &lt; b &amp;&amp; c &gt; d, &lt;script&gt;alert(1)&lt;/script&gt;"),
        "text stays text: {html}"
    );
    assert!(
        !html.contains("<script"),
        "escaped text never becomes an element: {html}"
    );
    let tags = start_tags(&html);
    let quoted = tag_before(&tags, &html, "Quoted");
    assert_eq!(
        attribute(&quoted.text, "title").as_deref(),
        Some("say &quot;hi&quot; &amp; go"),
        "an attribute value cannot break out of its quotes: {html}"
    );
    assert_eq!(result.created.len(), 3, "a div and two paragraphs");
}

// --- IDs on block-level elements ---

#[test]
fn every_block_level_element_gets_a_data_omni_id_and_inline_ones_do_not() {
    let (doc, result) = doc_of(RICH);
    let html = doc.to_html();
    let tags = start_tags(&html);
    for tag in &tags {
        if BLOCK_LEVEL.contains(&tag.name.as_str()) {
            let given = tag.omni_id().unwrap_or_else(|| {
                panic!(
                    "<{}> is block-level, so it has a data-omni-id: {}",
                    tag.name, tag.text
                )
            });
            assert!(
                is_node_id(&given),
                "<{}> has a valid node ID, not `{given}`",
                tag.name
            );
        }
        if NOT_BLOCK_LEVEL.contains(&tag.name.as_str()) {
            assert_eq!(
                tag.omni_id(),
                None,
                "<{}> is not block-level, so it has no data-omni-id: {}",
                tag.name,
                tag.text
            );
        }
    }
    assert!(
        !html.contains("data-node-id"),
        "the ID attribute is data-omni-id: {html}"
    );

    let printed = printed_ids(&html);
    let distinct: HashSet<&String> = printed.iter().collect();
    assert_eq!(
        distinct.len(),
        printed.len(),
        "every ID is used once: {html}"
    );
    let created: Vec<String> = result.created.iter().map(|node| node.to_string()).collect();
    assert_eq!(
        created, printed,
        "every element that got an ID is a new node, listed in document order"
    );
    assert_eq!(
        Some(created[0].as_str()),
        tags[0].omni_id().as_deref(),
        "the section is the inserted node"
    );
    assert_eq!(result.node.to_string(), created[0]);
    for node in &result.created {
        assert_eq!(doc.revision(*node), Some(1), "{node} is at revision 1");
    }
}

#[test]
fn the_ids_and_revisions_are_printed_and_read_back() {
    let (mut doc, result) = doc_of(RICH);
    let paragraph = result
        .created
        .iter()
        .copied()
        .find(|node| {
            doc.node_html(*node)
                .is_some_and(|html| html.starts_with("<p") && html.contains("Inside a card"))
        })
        .expect("the card's paragraph has an ID");
    apply(
        &mut doc,
        json!([{"op": "set", "node": paragraph.to_string(), "rev": 1, "properties": {"html": "Changed <em>here</em>"}}]),
    );

    let plain = doc.to_html();
    assert!(
        !plain.contains("data-rev"),
        "to_html has no revisions: {plain}"
    );
    let head = doc.to_html_with_revisions();
    let tags = start_tags(&head);
    let changed = tag_before(&tags, &head, "Changed ");
    assert_eq!(changed.omni_id(), Some(paragraph.to_string()), "{head}");
    assert_eq!(
        attribute(&changed.text, "data-rev").as_deref(),
        Some("2"),
        "the set paragraph is at revision 2: {head}"
    );

    let read = EditDoc::from_html(&head)
        .unwrap_or_else(|error| panic!("a printed head reads back: {error}\n{head}"));
    assert_eq!(
        read.to_html_with_revisions(),
        head,
        "and prints the same bytes"
    );
    assert_eq!(read.to_html(), plain);
    assert_eq!(read.revision(paragraph), Some(2));
    let before: Vec<_> = doc.index().into_iter().map(|n| (n.id, n.parent)).collect();
    let after: Vec<_> = read.index().into_iter().map(|n| (n.id, n.parent)).collect();
    assert_eq!(after, before, "the same nodes under the same parents");
}

#[test]
fn an_agent_supplied_valid_unique_id_is_kept() {
    let (doc, result) = doc_of(
        "<div data-omni-id=\"k3f9x2qa\"><p data-omni-id=\"k3f9x2qb\">Mine</p><p>New</p></div>",
    );
    assert_eq!(result.node, id("k3f9x2qa"), "the div keeps its ID");
    assert_eq!(result.created.len(), 3, "{:?}", result.created);
    assert_eq!(result.created[0], id("k3f9x2qa"));
    assert_eq!(
        result.created[1],
        id("k3f9x2qb"),
        "the paragraph keeps its ID"
    );
    assert!(
        !["k3f9x2qa", "k3f9x2qb"].contains(&result.created[2].to_string().as_str()),
        "the paragraph without one gets a new ID"
    );
    assert_eq!(
        doc.children(Some(id("k3f9x2qa"))),
        Some(result.created[1..].to_vec())
    );
    assert!(doc.to_html().contains("data-omni-id=\"k3f9x2qb\""));
}

#[test]
fn an_invalid_id_is_replaced_by_a_new_one() {
    let invalid = [
        "K3F9X2QC",
        "k3f9x2q",
        "k3f9x2qi",
        "k3f9x2qu1",
        "not an id",
        "",
    ];
    let content = format!(
        "<div>{}</div>",
        invalid
            .iter()
            .enumerate()
            .map(|(n, value)| format!("<p data-omni-id=\"{value}\">Invalid {n}.</p>"))
            .collect::<String>()
    );
    let (doc, result) = doc_of(&content);
    let html = doc.to_html();
    let tags = start_tags(&html);
    let mut seen = HashSet::new();
    for (n, value) in invalid.iter().enumerate() {
        let tag = tag_before(&tags, &html, &format!("Invalid {n}."));
        assert_eq!(tag.name, "p");
        let given = tag
            .omni_id()
            .unwrap_or_else(|| panic!("the paragraph with `{value}` still has an ID: {html}"));
        assert!(
            is_node_id(&given),
            "`{value}` was replaced by a valid ID, not `{given}`"
        );
        assert_ne!(given, *value, "`{value}` is not kept");
        assert!(seen.insert(given), "every new ID is used once: {html}");
    }
    for value in invalid.iter().filter(|value| !value.is_empty()) {
        assert!(!html.contains(*value), "`{value}` is gone: {html}");
    }
    assert_eq!(result.created.len(), 1 + invalid.len());
}

#[test]
fn an_id_used_twice_in_the_content_is_kept_only_the_first_time() {
    let (doc, result) = doc_of(
        "<div data-omni-id=\"k3f9x2qd\"><p data-omni-id=\"k3f9x2qd\">Second use</p><p data-omni-id=\"k3f9x2qe\">Unique</p><p data-omni-id=\"k3f9x2qe\">Third use</p></div>",
    );
    let html = doc.to_html();
    let tags = start_tags(&html);
    assert_eq!(result.node, id("k3f9x2qd"), "the first use keeps it");
    let second = tag_before(&tags, &html, "Second use")
        .omni_id()
        .expect("an ID");
    assert_ne!(second, "k3f9x2qd", "a second use gets a new ID: {html}");
    assert!(is_node_id(&second));
    assert_eq!(
        tag_before(&tags, &html, "Unique").omni_id().as_deref(),
        Some("k3f9x2qe")
    );
    let third = tag_before(&tags, &html, "Third use")
        .omni_id()
        .expect("an ID");
    assert_ne!(third, "k3f9x2qe", "{html}");
    let printed = printed_ids(&html);
    let distinct: HashSet<&String> = printed.iter().collect();
    assert_eq!(distinct.len(), 4, "four elements, four IDs: {html}");
}

#[test]
fn an_id_another_node_of_the_doc_has_is_not_taken() {
    let (mut doc, first) = doc_of(
        "<section data-omni-id=\"k3f9x2qf\"><p data-omni-id=\"k3f9x2qg\">Original</p></section>",
    );
    assert_eq!(first.node, id("k3f9x2qf"));
    let results = apply(
        &mut doc,
        json!([{"op": "insert", "parent": null, "position": {"after": "k3f9x2qf"}, "content": "<div data-omni-id=\"k3f9x2qf\"><p data-omni-id=\"k3f9x2qg\">Intruder</p></div>"}]),
    );
    let inserted = &results[0];
    assert_ne!(
        inserted.node,
        id("k3f9x2qf"),
        "the doc's own section keeps its ID"
    );
    assert!(
        !inserted.created.contains(&id("k3f9x2qg")),
        "the doc's own paragraph keeps its ID: {:?}",
        inserted.created
    );
    assert_eq!(inserted.created.len(), 2);
    assert_eq!(
        doc.children(None),
        Some(vec![id("k3f9x2qf"), inserted.node])
    );
    assert_eq!(
        doc.children(Some(id("k3f9x2qf"))),
        Some(vec![id("k3f9x2qg")])
    );
    assert!(
        doc.node_html(id("k3f9x2qg"))
            .expect("still there")
            .contains("Original")
    );
    assert_eq!(doc.revision(id("k3f9x2qf")), Some(1));
    assert_eq!(doc.revision(id("k3f9x2qg")), Some(1));
}

// --- The format document ---

#[test]
fn the_format_is_documented_with_examples_that_load() {
    let path = repo_root().join("docs/formats/html-docs.md");
    let doc = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "docs/formats/html-docs.md must document the HTML doc format, but it cannot be read ({}): {error}",
            path.display()
        )
    });
    let examples = html_fences(&doc);
    assert!(
        !examples.is_empty(),
        "docs/formats/html-docs.md must show an example doc in an ```html code block"
    );
    let mut failures = Vec::new();
    for example in &examples {
        match EditDoc::from_html(example) {
            Err(error) => failures.push(format!("{error}: {example}")),
            Ok(_) => {
                let tags = start_tags(example);
                let missing: Vec<&str> = tags
                    .iter()
                    .filter(|tag| {
                        BLOCK_LEVEL.contains(&tag.name.as_str()) && tag.omni_id().is_none()
                    })
                    .map(|tag| tag.text.as_str())
                    .collect();
                if !missing.is_empty() {
                    failures.push(format!(
                        "block-level elements without a data-omni-id: {missing:?}"
                    ));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "every ```html example in docs/formats/html-docs.md must load as a doc with a data-omni-id on every block-level element: {failures:?}"
    );
}

/// The contents of the ```html fenced code blocks of a Markdown file.
fn html_fences(markdown: &str) -> Vec<String> {
    let mut fences = Vec::new();
    let mut current: Option<String> = None;
    for line in markdown.lines() {
        let fence_line = line.trim_start().starts_with("```");
        match current.as_mut() {
            None if line.trim_start().starts_with("```html") => current = Some(String::new()),
            None => {}
            Some(_) if fence_line => fences.extend(current.take()),
            Some(fence) => {
                fence.push_str(line);
                fence.push('\n');
            }
        }
    }
    fences
}
