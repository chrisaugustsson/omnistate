//! US-020: the HTML doc format. `omni-core` parses a strict HTML form into a
//! tree of typed blocks with stable IDs, cleans it, stores it as JSON and
//! prints it back as HTML.
//!
//! The tests pin this HTML form (docs/formats/html-docs.md documents it):
//!
//! - A doc is a list of top-level blocks. Whitespace between blocks is
//!   ignored; text outside a text block is an error.
//! - `<section>` and `<article data-block="user-story">` hold blocks.
//!   `<h1>` to `<h6>`, `<p>`, `<blockquote>`, `<aside data-callout="note|warning|decision">`,
//!   `<p data-block="requirement">`, `<li>` and `<td>`/`<th>` hold inline
//!   content. `<hr>` is a divider. `<ul>` and `<ol>` hold `<li>` only, and
//!   `<table>` holds `<tr>` only, which hold `<td>` and `<th>` only.
//! - `<figure>` holds one `<svg>` or one `<pre class="mermaid">` and a
//!   `<figcaption>`. A code block is `<pre><code>`, text only. An image is
//!   `<img src="attachment:<id>" alt="...">`.
//! - Inline content is text with `<strong>`, `<em>`, `<code>`, `<a href>` and
//!   `<br>`; `<hr>` and `<br>` may be written `<hr/>` and `<br/>`.
//! - Any block may carry `data-node-id`.
//! - An error is a `ParseError` whose `line()` is the line of the offending
//!   tag and whose text reads `line <n>: <problem>`, naming the tag or
//!   attribute.

use omni_core::html_doc::{
    Block, BlockKind, CalloutKind, FigureSource, HtmlDoc, Inline, ParseError,
};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const ALPHABET: &[u8] = b"0123456789abcdefghjkmnpqrstvwxyz";

const PHOTO: &str = "attachment:0192f1c4-5b6a-7d8e-9f01-23456789abcd";

const ALL_KINDS: [BlockKind; 16] = [
    BlockKind::Section,
    BlockKind::Heading,
    BlockKind::Paragraph,
    BlockKind::Quote,
    BlockKind::Divider,
    BlockKind::List,
    BlockKind::ListItem,
    BlockKind::Table,
    BlockKind::TableRow,
    BlockKind::TableCell,
    BlockKind::Figure,
    BlockKind::Callout,
    BlockKind::CodeBlock,
    BlockKind::Image,
    BlockKind::Requirement,
    BlockKind::UserStory,
];

/// A doc with every kind of block. Each `data-node-id="@"` gets its own ID
/// from `with_ids`, or is dropped by `without_ids`.
const FULL_DOC: &str = r##"<section data-node-id="@">
<h1 data-node-id="@">Order service</h1>
<p data-node-id="@">Plain, <strong>strong</strong>, <em>emphasis</em>, <code>code()</code> and <a href="https://example.com/docs">a link</a>.<br>After the line break.</p>
<blockquote data-node-id="@">A quoted line.</blockquote>
<hr data-node-id="@">
<ul data-node-id="@">
<li data-node-id="@">First item</li>
<li data-node-id="@">Second item</li>
</ul>
<table data-node-id="@">
<tr data-node-id="@"><th data-node-id="@">Field</th><th data-node-id="@">Type</th></tr>
<tr data-node-id="@"><td data-node-id="@">id</td><td data-node-id="@">uuid</td></tr>
</table>
<figure data-node-id="@"><svg viewBox="0 0 100 50"><rect x="1" y="1" width="50" height="20" fill="#336699"/><circle cx="70" cy="25" r="10"/></svg><figcaption>A box and a circle</figcaption></figure>
<figure data-node-id="@"><pre class="mermaid">graph TD;
  A--&gt;B</pre><figcaption>The flow</figcaption></figure>
<aside data-node-id="@" data-callout="warning">Mind the gap.</aside>
<pre data-node-id="@"><code>fn main() {
    println!("hi");
}</code></pre>
<img data-node-id="@" src="attachment:0192f1c4-5b6a-7d8e-9f01-23456789abcd" alt="A screenshot">
<p data-node-id="@" data-block="requirement">FR-1: The system must keep every version.</p>
<article data-node-id="@" data-block="user-story">
<h3 data-node-id="@">Reset a password</h3>
<p data-node-id="@">As a user, I want to reset my password so that I can get back in.</p>
<ol data-node-id="@"><li data-node-id="@">Open the link</li></ol>
</article>
</section>
"##;

/// `template` with a distinct valid node ID in every `data-node-id="@"`.
fn with_ids(template: &str) -> String {
    let mut html = String::new();
    for (index, part) in template.split("data-node-id=\"@\"").enumerate() {
        if index > 0 {
            let digit = ALPHABET[index - 1] as char;
            html.push_str(&format!("data-node-id=\"b000000{digit}\""));
        }
        html.push_str(part);
    }
    html
}

/// `template` with every `data-node-id="@"` left out.
fn without_ids(template: &str) -> String {
    template.replace(" data-node-id=\"@\"", "")
}

fn parse(html: &str) -> HtmlDoc {
    HtmlDoc::parse(html).unwrap_or_else(|error| {
        panic!("this HTML doc must be accepted, but it was rejected: {error}\n{html}")
    })
}

fn rejected(html: &str, why: &str) -> ParseError {
    match HtmlDoc::parse(html) {
        Ok(doc) => panic!("{why}, but this HTML was accepted as {doc:?}:\n{html}"),
        Err(error) => error,
    }
}

/// Asserts that `html` is rejected, with an error that names `line` and
/// mentions `problem` (a tag or attribute name).
fn assert_rejected_at(html: &str, line: usize, problem: &str, why: &str) {
    let error = rejected(html, why);
    assert_eq!(
        error.line(),
        line,
        "{why}: the error must name line {line}, but it is: {error}\n{html}"
    );
    assert!(
        error.to_string().contains(&format!("line {line}")),
        "the error text must name the line, as `line {line}: ...`, but it is `{error}`"
    );
    assert!(
        names_problem(&error, problem),
        "{why}: the error must name `{problem}`, but it is `{error}`\n{html}"
    );
}

/// Whether `error` names the tag, attribute or word `problem`, or any of its
/// alternatives when it lists several as `a|b`. A name of one or two letters,
/// such as `b` or `td`, must be written as `<b>` or in quotes, since a plain
/// letter is in every message.
fn names_problem(error: &ParseError, problem: &str) -> bool {
    let message = error.message().to_lowercase();
    problem.split('|').any(|name| {
        if name.len() > 2 {
            message.contains(name)
        } else {
            [
                format!("<{name}"),
                format!("`{name}`"),
                format!("'{name}'"),
                format!("\"{name}\""),
            ]
            .iter()
            .any(|form| message.contains(form.as_str()))
        }
    })
}

fn walk<'a>(blocks: &'a [Block], out: &mut Vec<&'a Block>) {
    for block in blocks {
        out.push(block);
        walk(block.children(), out);
    }
}

fn all_blocks(doc: &HtmlDoc) -> Vec<&Block> {
    let mut out = Vec::new();
    walk(doc.blocks(), &mut out);
    out
}

fn kinds(blocks: &[Block]) -> Vec<BlockKind> {
    blocks.iter().map(Block::kind).collect()
}

fn first_of(doc: &HtmlDoc, kind: BlockKind) -> &Block {
    all_blocks(doc)
        .into_iter()
        .find(|block| block.kind() == kind)
        .unwrap_or_else(|| panic!("the doc must have a {kind:?} block"))
}

/// The block that follows `html` wrapped in nothing: a one-block doc.
fn only_block(html: &str) -> Block {
    let doc = parse(html);
    assert_eq!(doc.blocks().len(), 1, "expected one block in {html}");
    doc.blocks()[0].clone()
}

fn text(content: &str) -> Inline {
    Inline::Text(content.to_string())
}

/// The `data-node-id` values of printed HTML, in order.
fn printed_ids(html: &str) -> Vec<String> {
    const ATTRIBUTE: &str = "data-node-id=\"";
    let mut ids = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find(ATTRIBUTE) {
        rest = &rest[start + ATTRIBUTE.len()..];
        let end = rest.find('"').expect("an attribute value is closed");
        ids.push(rest[..end].to_string());
        rest = &rest[end..];
    }
    ids
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

// --- Typed blocks ---

#[test]
fn a_doc_with_every_kind_of_block_parses_into_a_tree_of_typed_blocks() {
    let doc = parse(&with_ids(FULL_DOC));
    let found: HashSet<BlockKind> = all_blocks(&doc).iter().map(|block| block.kind()).collect();
    for kind in ALL_KINDS {
        assert!(found.contains(&kind), "the tree must have a {kind:?} block");
    }
    assert_eq!(doc.blocks().len(), 1, "the doc is one section");
    assert_eq!(doc.blocks()[0].kind(), BlockKind::Section);
    assert_eq!(
        kinds(doc.blocks()[0].children()),
        [
            BlockKind::Heading,
            BlockKind::Paragraph,
            BlockKind::Quote,
            BlockKind::Divider,
            BlockKind::List,
            BlockKind::Table,
            BlockKind::Figure,
            BlockKind::Figure,
            BlockKind::Callout,
            BlockKind::CodeBlock,
            BlockKind::Image,
            BlockKind::Requirement,
            BlockKind::UserStory,
        ],
        "the section's children are its blocks, in order"
    );
}

#[test]
fn sections_nest_and_headings_have_levels() {
    let doc = parse(
        "<section><h1>One</h1><section><h2>Two</h2><p>Text</p></section></section>\n<h6>Six</h6>\n<hr>\n<p>Last</p>",
    );
    assert_eq!(
        kinds(doc.blocks()),
        [
            BlockKind::Section,
            BlockKind::Heading,
            BlockKind::Divider,
            BlockKind::Paragraph
        ]
    );
    let outer = &doc.blocks()[0];
    assert_eq!(
        kinds(outer.children()),
        [BlockKind::Heading, BlockKind::Section]
    );
    assert_eq!(
        kinds(outer.children()[1].children()),
        [BlockKind::Heading, BlockKind::Paragraph]
    );
    assert_eq!(outer.children()[0].heading_level(), Some(1));
    assert_eq!(outer.children()[1].children()[0].heading_level(), Some(2));
    assert_eq!(doc.blocks()[1].heading_level(), Some(6));
    for level in 1..=6 {
        let heading = only_block(&format!("<h{level}>Title</h{level}>"));
        assert_eq!(heading.kind(), BlockKind::Heading);
        assert_eq!(
            heading.heading_level(),
            Some(level),
            "<h{level}> is a level {level} heading"
        );
        assert_eq!(heading.inline(), [text("Title")]);
    }
    assert_eq!(doc.blocks()[3].inline(), [text("Last")]);
    assert_eq!(doc.blocks()[2].inline(), [], "a divider has no content");
    for divider in ["<hr>", "<hr/>", "<hr />"] {
        assert_eq!(
            only_block(divider).kind(),
            BlockKind::Divider,
            "{divider} is a divider"
        );
    }
}

#[test]
fn quotes_keep_their_text() {
    let quote = only_block("<blockquote>Measure twice, cut once.</blockquote>");
    assert_eq!(quote.kind(), BlockKind::Quote);
    assert_eq!(quote.inline(), [text("Measure twice, cut once.")]);
}

#[test]
fn lists_hold_list_items_and_are_ordered_or_not() {
    let bulleted = only_block("<ul><li>One</li><li>Two</li></ul>");
    assert_eq!(bulleted.kind(), BlockKind::List);
    assert!(!bulleted.is_ordered(), "<ul> is a bulleted list");
    assert_eq!(
        kinds(bulleted.children()),
        [BlockKind::ListItem, BlockKind::ListItem]
    );
    assert_eq!(bulleted.children()[1].inline(), [text("Two")]);

    let numbered = only_block("<ol><li>One</li></ol>");
    assert_eq!(numbered.kind(), BlockKind::List);
    assert!(numbered.is_ordered(), "<ol> is a numbered list");
    assert_eq!(kinds(numbered.children()), [BlockKind::ListItem]);
}

#[test]
fn tables_hold_rows_that_hold_cells() {
    let table = only_block(
        "<table>\n<tr><th>Field</th><th>Type</th></tr>\n<tr><td>id</td><td>uuid</td></tr>\n</table>",
    );
    assert_eq!(table.kind(), BlockKind::Table);
    assert_eq!(
        kinds(table.children()),
        [BlockKind::TableRow, BlockKind::TableRow]
    );
    let header = &table.children()[0];
    let body = &table.children()[1];
    assert_eq!(
        kinds(header.children()),
        [BlockKind::TableCell, BlockKind::TableCell]
    );
    assert!(
        header.children().iter().all(Block::is_header),
        "<th> cells are header cells"
    );
    assert!(
        body.children().iter().all(|cell| !cell.is_header()),
        "<td> cells are not"
    );
    assert_eq!(header.children()[0].inline(), [text("Field")]);
    assert_eq!(body.children()[1].inline(), [text("uuid")]);
}

#[test]
fn figures_show_inline_svg_or_mermaid_code_with_a_caption() {
    let svg = only_block(
        r#"<figure><svg viewBox="0 0 10 10"><rect width="5" height="5"/></svg><figcaption>A square</figcaption></figure>"#,
    );
    assert_eq!(svg.kind(), BlockKind::Figure);
    match svg.figure() {
        Some(FigureSource::Svg(markup)) => {
            assert!(markup.contains("<svg"), "the SVG markup is kept: {markup}");
            assert!(
                markup.contains("<rect"),
                "the SVG shapes are kept: {markup}"
            );
        }
        other => panic!("a figure with an <svg> shows SVG, not {other:?}"),
    }
    assert_eq!(svg.caption(), Some("A square"));

    let mermaid = only_block(
        "<figure><pre class=\"mermaid\">graph TD;\n  A--&gt;B</pre><figcaption>The flow</figcaption></figure>",
    );
    assert_eq!(mermaid.kind(), BlockKind::Figure);
    assert_eq!(
        mermaid.figure(),
        Some(&FigureSource::Mermaid("graph TD;\n  A-->B".to_string())),
        "Mermaid code is kept as text, exactly as written and without entities"
    );
    assert_eq!(mermaid.caption(), Some("The flow"));

    let unescaped = only_block(
        "<figure><pre class=\"mermaid\">A--&gt;B</pre><figcaption>c</figcaption></figure>",
    );
    assert_eq!(
        unescaped.figure(),
        Some(&FigureSource::Mermaid("A-->B".to_string()))
    );
}

#[test]
fn a_figure_needs_exactly_one_picture_and_a_caption() {
    assert_rejected_at(
        "<p>One</p>\n<figure><svg viewBox=\"0 0 1 1\"></svg></figure>",
        2,
        "caption|figcaption",
        "a figure without a caption is rejected",
    );
    assert_rejected_at(
        "<p>One</p>\n<figure><figcaption>Nothing to see</figcaption></figure>",
        2,
        "figure|svg|mermaid",
        "a figure without an SVG or Mermaid code is rejected",
    );
}

#[test]
fn callouts_are_notes_warnings_or_decisions() {
    for (attribute, kind) in [
        ("note", CalloutKind::Note),
        ("warning", CalloutKind::Warning),
        ("decision", CalloutKind::Decision),
    ] {
        let callout = only_block(&format!(
            "<aside data-callout=\"{attribute}\">Read this.</aside>"
        ));
        assert_eq!(callout.kind(), BlockKind::Callout);
        assert_eq!(
            callout.callout_kind(),
            Some(kind),
            "data-callout=\"{attribute}\""
        );
        assert_eq!(callout.inline(), [text("Read this.")]);
    }
    assert_rejected_at(
        "<p>One</p>\n<aside data-callout=\"tip\">Read this.</aside>",
        2,
        "tip",
        "a callout that is not a note, warning or decision is rejected",
    );
    assert_rejected_at(
        "<p>One</p>\n<aside>Read this.</aside>",
        2,
        "data-callout|callout",
        "a callout without a kind is rejected",
    );
}

#[test]
fn code_blocks_keep_their_text_exactly() {
    let source = "fn main() {\n    if a &lt; b &amp;&amp; c {\n        run();\n    }\n}";
    let block = only_block(&format!("<pre><code>{source}</code></pre>"));
    assert_eq!(block.kind(), BlockKind::CodeBlock);
    assert_eq!(
        block.code(),
        Some("fn main() {\n    if a < b && c {\n        run();\n    }\n}"),
        "code keeps its line breaks and indentation, with entities decoded"
    );
    assert_eq!(block.inline(), [], "a code block has no inline markup");
}

#[test]
fn images_show_an_attachment() {
    let image = only_block(&format!("<img src=\"{PHOTO}\" alt=\"A screenshot\">"));
    assert_eq!(image.kind(), BlockKind::Image);
    assert_eq!(image.attachment().map(|a| a.as_str()), Some(PHOTO));
    assert_eq!(image.alt(), Some("A screenshot"));
    assert_eq!(
        only_block(&format!("<img src=\"{PHOTO}\" alt=\"x\"/>")).kind(),
        BlockKind::Image
    );
}

#[test]
fn requirements_and_user_stories_are_blocks_of_their_own() {
    let requirement = only_block(
        "<p data-block=\"requirement\">FR-1: The system must keep <strong>every</strong> version.</p>",
    );
    assert_eq!(requirement.kind(), BlockKind::Requirement);
    assert_eq!(
        requirement.inline(),
        [
            text("FR-1: The system must keep "),
            Inline::Strong(vec![text("every")]),
            text(" version.")
        ]
    );
    assert_eq!(
        only_block("<p>FR-1: Plain.</p>").kind(),
        BlockKind::Paragraph
    );

    let story = only_block(
        "<article data-block=\"user-story\">\n<h3>Reset a password</h3>\n<p>As a user, I want to reset my password.</p>\n<ul><li>The link works once</li></ul>\n</article>",
    );
    assert_eq!(story.kind(), BlockKind::UserStory);
    assert_eq!(
        kinds(story.children()),
        [BlockKind::Heading, BlockKind::Paragraph, BlockKind::List]
    );
    assert_rejected_at(
        "<p>One</p>\n<p data-block=\"bogus\">Text</p>",
        2,
        "bogus",
        "an unknown block type is rejected",
    );
}

// --- Inline markup ---

#[test]
fn text_blocks_allow_strong_emphasis_code_links_and_line_breaks() {
    let paragraph = only_block(
        "<p>Plain <strong>bold</strong> <em>slanted</em> <code>x &lt; y</code> <a href=\"https://example.com/a?b=1&amp;c=2\">a link</a><br>next<br/>last</p>",
    );
    assert_eq!(
        paragraph.inline(),
        [
            text("Plain "),
            Inline::Strong(vec![text("bold")]),
            text(" "),
            Inline::Emphasis(vec![text("slanted")]),
            text(" "),
            Inline::Code("x < y".to_string()),
            text(" "),
            Inline::Link {
                href: "https://example.com/a?b=1&c=2".to_string(),
                children: vec![text("a link")]
            },
            Inline::LineBreak,
            text("next"),
            Inline::LineBreak,
            text("last"),
        ]
    );
    let nested = only_block("<p><strong>bold and <em>both</em></strong></p>");
    assert_eq!(
        nested.inline(),
        [Inline::Strong(vec![
            text("bold and "),
            Inline::Emphasis(vec![text("both")])
        ])]
    );
}

#[test]
fn every_text_block_takes_inline_markup() {
    let blocks = [
        "<h2>A <strong>bold</strong> title</h2>",
        "<p>A <strong>bold</strong> word</p>",
        "<blockquote>A <strong>bold</strong> word</blockquote>",
        "<ul><li>A <strong>bold</strong> word</li></ul>",
        "<table><tr><td>A <strong>bold</strong> word</td></tr></table>",
        "<aside data-callout=\"note\">A <strong>bold</strong> word</aside>",
        "<p data-block=\"requirement\">A <strong>bold</strong> word</p>",
    ];
    for html in blocks {
        let doc = parse(html);
        let has_strong = all_blocks(&doc).iter().any(|block| {
            block
                .inline()
                .iter()
                .any(|part| matches!(part, Inline::Strong(_)))
        });
        assert!(has_strong, "inline <strong> must be kept in {html}");
    }
}

#[test]
fn text_is_unescaped_when_read_and_escaped_when_printed() {
    let hostile =
        "<p data-node-id=\"b0000001\">&lt;script&gt;alert(1)&lt;/script&gt; 1 &lt; 2 &amp; 3</p>";
    let doc = parse(hostile);
    assert_eq!(
        doc.blocks()[0].inline(),
        [text("<script>alert(1)</script> 1 < 2 & 3")],
        "escaped markup is only text"
    );
    let html = doc.to_html();
    assert!(
        !html.contains("<script"),
        "printing escapes text, so no tag appears: {html}"
    );
    assert!(
        html.contains("1 &lt; 2 &amp; 3"),
        "printing escapes < and &: {html}"
    );
    assert_eq!(
        parse(&html),
        doc,
        "the printed text parses back to the same doc"
    );
}

#[test]
fn attribute_values_cannot_break_out_when_printed() {
    let html = format!(
        "<img data-node-id=\"b0000001\" src=\"{PHOTO}\" alt=\"say &quot;hi&quot; &lt;b&gt; &amp; go\">\n<p data-node-id=\"b0000002\"><a href=\"https://example.com/?a=1&amp;b=%22x%22\">link</a></p>"
    );
    let doc = parse(&html);
    assert_eq!(doc.blocks()[0].alt(), Some("say \"hi\" <b> & go"));
    let printed = doc.to_html();
    assert!(
        !printed.contains("<b>"),
        "printed attribute values are escaped: {printed}"
    );
    assert_eq!(
        parse(&printed),
        doc,
        "the printed attributes parse back to the same doc"
    );
}

#[test]
fn inline_tags_outside_the_list_are_rejected() {
    for (tag, html) in [
        ("span", "<p>one</p>\n<p>some <span>text</span></p>"),
        ("b", "<p>one</p>\n<p>some <b>text</b></p>"),
        ("i", "<p>one</p>\n<p>some <i>text</i></p>"),
        ("u", "<p>one</p>\n<p>some <u>text</u></p>"),
        ("mark", "<p>one</p>\n<p>some <mark>text</mark></p>"),
        (
            "img",
            format!("<p>one</p>\n<p>some <img src=\"{PHOTO}\" alt=\"x\"></p>").as_str(),
        ),
        (
            "script",
            "<p>one</p>\n<p>some <script>alert(1)</script></p>",
        ),
        (
            "span",
            "<p>one</p>\n<h2>a <strong>b <span>c</span></strong></h2>",
        ),
        ("span", "<p>one</p>\n<ul><li>a <span>c</span></li></ul>"),
        (
            "span",
            "<p>one</p>\n<table><tr><td>a <span>c</span></td></tr></table>",
        ),
        ("b", "<p>one</p>\n<pre><code>a <b>c</b></code></pre>"),
        (
            "b",
            "<p>one</p>\n<figure><pre class=\"mermaid\">a <b>c</b></pre><figcaption>x</figcaption></figure>",
        ),
    ] {
        assert_rejected_at(html, 2, tag, &format!("<{tag}> is not inline markup"));
    }
}

// --- Rejection with the line and the problem ---

#[test]
fn any_other_tag_is_rejected_with_its_line_and_name() {
    for tag in [
        "script", "div", "span", "iframe", "b", "i", "u", "font", "style", "form", "input",
        "button", "details", "nav", "main", "center", "marquee", "object", "embed", "svg", "a",
        "h7", "tbody", "dl", "video", "link", "meta", "textarea",
    ] {
        let html = format!("<p>one</p>\n<p>two</p>\n<{tag}>x</{tag}>\n<p>four</p>");
        assert_rejected_at(&html, 3, tag, &format!("<{tag}> is not part of the format"));
    }
    assert_rejected_at(
        "<p>one</p>\n<article>no block type</article>",
        2,
        "article",
        "an <article> that is not a user story is rejected",
    );
}

#[test]
fn free_form_nesting_is_rejected_with_its_line_and_name() {
    for (tag, html, line) in [
        ("ul", "<p>one</p>\n<p>text\n<ul><li>x</li></ul></p>", 3),
        ("h2", "<p>one</p>\n<p>\n<h2>x</h2></p>", 3),
        ("p", "<p>one</p>\n<ul>\n<p>x</p>\n</ul>", 3),
        ("div", "<p>one</p>\n<ul>\n<div>x</div>\n</ul>", 3),
        ("li", "<p>one</p>\n<li>stray item</li>", 2),
        (
            "li",
            "<p>one</p>\n<section>\n<li>stray item</li>\n</section>",
            3,
        ),
        ("td", "<p>one</p>\n<table>\n<td>x</td>\n</table>", 3),
        ("tr", "<p>one</p>\n<tr><td>x</td></tr>", 2),
        ("td", "<p>one</p>\n<p>\n<td>x</td></p>", 3),
        (
            "table",
            "<p>one</p>\n<table><tr><td>\n<table><tr><td>x</td></tr></table>\n</td></tr></table>",
            3,
        ),
        ("p", "<p>one</p>\n<table><tr>\n<p>x</p>\n</tr></table>", 3),
        (
            "section",
            "<p>one</p>\n<ul><li>\n<section><p>x</p></section></li></ul>",
            3,
        ),
        (
            "figure",
            "<p>one</p>\n<p>\n<figure><svg></svg><figcaption>c</figcaption></figure></p>",
            3,
        ),
        ("strong", "<p>one</p>\n<strong>loose</strong>", 2),
        (
            "a",
            "<p>one</p>\n<a href=\"https://example.com\">loose</a>",
            2,
        ),
    ] {
        assert_rejected_at(html, line, tag, &format!("<{tag}> may not be nested here"));
    }
}

#[test]
fn text_outside_a_text_block_is_rejected() {
    assert_rejected_at(
        "<p>one</p>\nloose text\n<p>three</p>",
        2,
        "text|character|content",
        "text between top-level blocks is rejected",
    );
    assert_rejected_at(
        "<p>one</p>\n<ul>\nloose text\n<li>x</li>\n</ul>",
        3,
        "text|character|content",
        "text directly inside a list is rejected",
    );
}

#[test]
fn a_style_attribute_is_rejected_with_its_line() {
    for html in [
        "<p>one</p>\n<p style=\"color: red\">x</p>",
        "<p>one</p>\n<p>some <strong style=\"color: red\">x</strong></p>",
        "<p>one</p>\n<section style=\"display: none\"><p>x</p></section>",
        "<p>one</p>\n<h2 style=\"font-size: 40px\">x</h2>",
        "<p>one</p>\n<table style=\"width: 100%\"><tr><td>x</td></tr></table>",
        "<p>one</p>\n<table><tr><td style=\"color: red\">x</td></tr></table>",
        "<p>one</p>\n<ul><li style=\"color: red\">x</li></ul>",
        "<p>one</p>\n<hr style=\"height: 9px\">",
        "<p>one</p>\n<aside data-callout=\"note\" style=\"color: red\">x</aside>",
        &format!("<p>one</p>\n<img src=\"{PHOTO}\" alt=\"x\" style=\"width: 9px\">"),
        "<p>one</p>\n<p>some <a href=\"https://example.com\" style=\"color: red\">x</a></p>",
        "<p>one</p>\n<pre style=\"color: red\"><code>x</code></pre>",
        "<p>one</p>\n<figure style=\"color: red\"><svg></svg><figcaption>c</figcaption></figure>",
        "<p>one</p>\n<p STYLE=\"color: red\">x</p>",
    ] {
        assert_rejected_at(html, 2, "style", "a style attribute is rejected");
    }
}

#[test]
fn event_handler_attributes_are_rejected_with_their_line() {
    for (name, html) in [
        ("onclick", "<p>one</p>\n<p onclick=\"alert(1)\">x</p>"),
        (
            "onmouseover",
            "<p>one</p>\n<p>some <strong onmouseover=\"alert(1)\">x</strong></p>",
        ),
        (
            "onerror",
            "<p>one</p>\n<img src=\"attachment:0192f1c4-5b6a-7d8e-9f01-23456789abcd\" alt=\"x\" onerror=\"alert(1)\">",
        ),
        (
            "onclick",
            "<p>one</p>\n<p>some <a href=\"https://example.com\" onclick=\"alert(1)\">x</a></p>",
        ),
    ] {
        assert_rejected_at(html, 2, name, &format!("a {name} attribute is rejected"));
    }
}

#[test]
fn the_first_problem_names_its_line() {
    let html = "<p>one</p>\n\n<ul>\n<li>fine</li>\n</ul>\n<p>five</p>\n<span>seven</span>\n<div>eight</div>";
    assert_rejected_at(html, 7, "span", "the first bad tag is the one reported");
}

// --- Node IDs ---

#[test]
fn blocks_may_carry_a_node_id() {
    let doc = parse(&with_ids(FULL_DOC));
    let blocks = all_blocks(&doc);
    let mut seen = HashSet::new();
    for block in &blocks {
        let id = block
            .id()
            .unwrap_or_else(|| panic!("a {:?} block carries its data-node-id", block.kind()));
        assert!(
            !block.is_new(),
            "a {:?} block with an ID is not new",
            block.kind()
        );
        assert!(
            seen.insert(id.to_string()),
            "{id} is read for one block only"
        );
    }
    let section = &doc.blocks()[0];
    assert_eq!(section.id().map(|id| id.as_str()), Some("b0000000"));
}

#[test]
fn blocks_without_a_node_id_are_marked_new() {
    let doc = parse(&without_ids(FULL_DOC));
    for block in all_blocks(&doc) {
        assert!(
            block.is_new(),
            "a {:?} block without data-node-id is new",
            block.kind()
        );
        assert!(
            block.id().is_none(),
            "a new {:?} block has no ID",
            block.kind()
        );
    }
    let mixed = parse(
        "<section data-node-id=\"k3f9x2qa\">\n<p data-node-id=\"m4g8y3rb\">Kept</p>\n<p>Added</p>\n</section>",
    );
    let section = &mixed.blocks()[0];
    assert!(!section.is_new());
    assert_eq!(section.id().map(|id| id.as_str()), Some("k3f9x2qa"));
    assert!(!section.children()[0].is_new());
    assert_eq!(
        section.children()[0].id().map(|id| id.as_str()),
        Some("m4g8y3rb")
    );
    assert!(
        section.children()[1].is_new(),
        "a block without an ID among blocks with IDs is new"
    );
}

#[test]
fn a_node_id_that_is_not_one_is_rejected_with_its_line() {
    for id in [
        "K3F9X2QA",
        "k3f9x2q",
        "k3f9x2qa0",
        "",
        "k3f9x2qi",
        "k3f9-2qa",
    ] {
        let html = format!("<p>one</p>\n<p data-node-id=\"{id}\">x</p>");
        assert_rejected_at(
            &html,
            2,
            "data-node-id|node id",
            &format!("`{id}` is not a node ID"),
        );
    }
}

// --- Cleaning ---

#[test]
fn inline_svg_is_reduced_to_an_allow_list() {
    let dirty = [
        r##"<svg viewBox="0 0 10 10"><script>alert(1)</script><rect width="5" height="5" onclick="alert(2)" onload="alert(2)"/><foreignObject width="10" height="10"><body xmlns="http://www.w3.org/1999/xhtml"><p>alert(4)</p></body></foreignObject><image href="https://evil.example/a.png" width="5" height="5"/><use xlink:href="https://evil.example/s.svg#a"/><use href="https://evil.example/t.svg#a"/><a href="javascript:alert(3)"><path d="M0 0L5 5"/></a><style>@import url(https://evil.example/x.css);</style><rect width="3" height="3" style="fill: url(https://evil.example/b.svg#f)"/><circle cx="5" cy="5" r="2"/></svg>"##,
        r##"<svg viewBox="0 0 10 10"><ScRiPt>alert(1)</ScRiPt><rect width="5" height="5" ONCLICK="alert(2)" OnLoad="alert(2)"/><FOREIGNOBJECT width="10" height="10"><p>alert(4)</p></FOREIGNOBJECT><circle cx="5" cy="5" r="2"/></svg>"##,
    ];
    for svg in dirty {
        let html = format!(
            "<figure data-node-id=\"b0000001\">{svg}<figcaption>Cleaned</figcaption></figure>"
        );
        let doc = parse(&html);
        let figure = &doc.blocks()[0];
        let stored = match figure.figure() {
            Some(FigureSource::Svg(markup)) => markup.clone(),
            other => panic!("the figure shows the cleaned SVG, not {other:?}"),
        };
        let json = serde_json::to_string(&doc).expect("a doc serializes");
        for (what, output) in [
            ("the stored SVG", stored.clone()),
            ("the printed doc", doc.to_html()),
            ("the stored JSON", json),
        ] {
            let lower = output.to_lowercase();
            for forbidden in [
                "<script",
                "alert",
                "onclick",
                "onload",
                "foreignobject",
                "evil.example",
                "javascript:",
                "@import",
            ] {
                assert!(
                    !lower.contains(forbidden),
                    "{what} must not contain `{forbidden}`: {output}"
                );
            }
        }
        let lower = stored.to_lowercase();
        assert!(
            lower.contains("<svg"),
            "the cleaned SVG is still an SVG: {stored}"
        );
        assert!(lower.contains("<rect"), "allowed shapes are kept: {stored}");
        assert!(
            lower.contains("<circle"),
            "allowed shapes after removed ones are kept: {stored}"
        );
        assert_eq!(figure.caption(), Some("Cleaned"));
    }
}

#[test]
fn clean_inline_svg_keeps_its_shapes_and_text() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 60"><g fill="none" stroke="#333"><rect x="1" y="1" width="40" height="20" rx="3"/><ellipse cx="70" cy="30" rx="20" ry="10"/><line x1="0" y1="50" x2="100" y2="50"/><polyline points="0,0 10,10 20,0"/><polygon points="0,0 10,10 20,0"/><path d="M0 0L10 10"/></g><text x="5" y="55">Label</text></svg>"##;
    let doc = parse(&format!(
        "<figure>{svg}<figcaption>Shapes</figcaption></figure>"
    ));
    let stored = match doc.blocks()[0].figure() {
        Some(FigureSource::Svg(markup)) => markup.to_lowercase(),
        other => panic!("the figure shows SVG, not {other:?}"),
    };
    for kept in [
        "<g",
        "<rect",
        "<ellipse",
        "<line",
        "<polyline",
        "<polygon",
        "<path",
        "<text",
        "label",
    ] {
        assert!(
            stored.contains(kept),
            "the allow-list must keep `{kept}`: {stored}"
        );
    }
}

#[test]
fn links_must_use_http_https_or_mailto() {
    for href in [
        "http://example.com/a",
        "https://example.com/a?b=1",
        "mailto:art@example.com",
    ] {
        let paragraph = only_block(&format!("<p><a href=\"{href}\">link</a></p>"));
        assert_eq!(
            paragraph.inline(),
            [Inline::Link {
                href: href.to_string(),
                children: vec![text("link")]
            }],
            "{href} is a valid link"
        );
    }
    for href in [
        "javascript:alert(1)",
        "JaVaScRiPt:alert(1)",
        "  javascript:alert(1)",
        "data:text/html,<p>x</p>",
        "vbscript:x",
        "file:///etc/passwd",
        "ftp://example.com/a",
        "/relative/path",
        "//example.com/a",
        "#fragment",
        "example.com",
        "",
    ] {
        let html = format!("<p>one</p>\n<p>see <a href=\"{href}\">link</a></p>");
        assert_rejected_at(
            &html,
            2,
            "href|link",
            &format!("a link to `{href}` is rejected"),
        );
    }
}

#[test]
fn images_must_be_attachments() {
    for src in [
        "https://example.com/a.png",
        "http://example.com/a.png",
        "data:image/png;base64,iVBORw0KGgo=",
        "/relative/a.png",
        "a.png",
        "attachment:",
        "attachment:not-an-id",
        "attachments:0192f1c4-5b6a-7d8e-9f01-23456789abcd",
        "javascript:alert(1)",
    ] {
        let html = format!("<p>one</p>\n<img src=\"{src}\" alt=\"x\">");
        assert_rejected_at(
            &html,
            2,
            "src|attachment|image",
            &format!("an image from `{src}` is rejected"),
        );
    }
    assert_rejected_at(
        "<p>one</p>\n<img alt=\"x\">",
        2,
        "src|attachment|image",
        "an image without a source is rejected",
    );
}

// --- Storage and printing ---

#[test]
fn a_doc_is_stored_as_its_block_tree_in_json_and_never_as_html() {
    let template = FULL_DOC
        .replace("<svg viewBox=\"0 0 100 50\"><rect x=\"1\" y=\"1\" width=\"50\" height=\"20\" fill=\"#336699\"/><circle cx=\"70\" cy=\"25\" r=\"10\"/></svg>", "<pre class=\"mermaid\">graph LR;\n  X--Y</pre>")
        .replace("A--&gt;B", "A--B");
    let doc = parse(&with_ids(&template));
    let json = serde_json::to_string(&doc).expect("a doc serializes to JSON");
    assert!(
        !json.contains('<') && !json.contains('>'),
        "the stored doc must hold no raw HTML: {json}"
    );

    let value: Value = serde_json::from_str(&json).expect("the stored doc is JSON");
    assert!(
        value.is_object() || value.is_array(),
        "the stored doc is a tree, not a string: {json}"
    );
    let mut strings = Vec::new();
    collect_strings(&value, &mut strings);
    for block in all_blocks(&doc) {
        let id = block.id().expect("every block has an ID").as_str();
        assert!(
            strings.iter().any(|s| s == id),
            "the stored tree holds the ID {id}: {json}"
        );
    }
    for words in [
        "Order service",
        "Mind the gap.",
        "Second item",
        "Reset a password",
    ] {
        assert!(
            strings.iter().any(|s| s.contains(words)),
            "the stored tree holds the text `{words}`"
        );
    }

    let loaded: HtmlDoc = serde_json::from_str(&json).expect("a stored doc loads");
    assert_eq!(
        loaded, doc,
        "loading the stored JSON gives back the same doc"
    );
    assert_eq!(loaded.to_html(), doc.to_html());
}

fn collect_strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => out.push(text.clone()),
        Value::Array(items) => items.iter().for_each(|item| collect_strings(item, out)),
        Value::Object(map) => map.values().for_each(|item| collect_strings(item, out)),
        _ => {}
    }
}

#[test]
fn a_stored_doc_keeps_new_blocks_new_and_svg_figures_intact() {
    let doc = parse(&without_ids(FULL_DOC));
    let json = serde_json::to_string(&doc).expect("a doc serializes");
    let loaded: HtmlDoc = serde_json::from_str(&json).expect("a stored doc loads");
    assert_eq!(loaded, doc);
    assert!(
        all_blocks(&loaded).iter().all(|block| block.is_new()),
        "blocks without IDs are still new after storing"
    );
    assert_eq!(
        first_of(&loaded, BlockKind::CodeBlock).code(),
        first_of(&doc, BlockKind::CodeBlock).code()
    );
    assert_eq!(
        loaded.blocks()[0].children()[6].figure(),
        doc.blocks()[0].children()[6].figure(),
        "the SVG figure survives storage"
    );
}

#[test]
fn printing_a_doc_puts_data_node_id_on_every_block() {
    let doc = parse(&with_ids(FULL_DOC));
    let blocks = all_blocks(&doc);
    let html = doc.to_html();
    let printed = printed_ids(&html);
    let expected: Vec<String> = blocks
        .iter()
        .map(|block| block.id().expect("every block has an ID").to_string())
        .collect();
    assert_eq!(
        printed, expected,
        "every block, and nothing else, is printed with its data-node-id, in document order:\n{html}"
    );
    assert!(blocks.len() >= 24, "the doc has many blocks");
    assert!(
        !html.contains("style="),
        "printed HTML has no style attribute: {html}"
    );
}

#[test]
fn printed_html_parses_back_to_the_same_doc() {
    let doc = parse(&with_ids(FULL_DOC));
    let html = doc.to_html();
    assert_eq!(
        parse(&html),
        doc,
        "the printed doc must parse back to the same tree:\n{html}"
    );
    assert_eq!(parse(&html).to_html(), html, "printing is stable");
    for markup in [
        "<strong>strong</strong>",
        "<em>emphasis</em>",
        "<code>code()</code>",
        "<a href=\"https://example.com/docs\">a link</a>",
        "<br",
    ] {
        assert!(
            html.contains(markup),
            "the printed doc keeps inline `{markup}`:\n{html}"
        );
    }
    assert!(
        html.contains(PHOTO),
        "the printed image shows its attachment:\n{html}"
    );
}

// --- The format document ---

#[test]
fn the_format_is_documented_with_a_complete_example() {
    let path = repo_root().join("docs/formats/html-docs.md");
    let doc = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!("docs/formats/html-docs.md must document the HTML doc format, but it cannot be read ({}): {error}", path.display())
    });
    for word in [
        "data-node-id",
        "section",
        "heading",
        "paragraph",
        "quote",
        "divider",
        "list",
        "table",
        "figure",
        "svg",
        "mermaid",
        "callout",
        "note",
        "warning",
        "decision",
        "code",
        "image",
        "attachment:",
        "requirement",
        "user-story",
        "strong",
        "emphasis",
        "http",
        "https",
        "mailto",
        "style",
        "line",
    ] {
        assert!(
            doc.to_lowercase().contains(word),
            "docs/formats/html-docs.md must cover `{word}`"
        );
    }

    let examples = html_fences(&doc);
    assert!(
        !examples.is_empty(),
        "docs/formats/html-docs.md must show its example in an ```html code block"
    );
    let mut failures = Vec::new();
    for example in &examples {
        match HtmlDoc::parse(example) {
            Err(error) => failures.push(error.to_string()),
            Ok(parsed) => {
                let blocks = all_blocks(&parsed);
                let found: HashSet<BlockKind> = blocks.iter().map(|block| block.kind()).collect();
                let callouts: HashSet<_> = blocks
                    .iter()
                    .filter_map(|block| block.callout_kind())
                    .collect();
                let inline = format!(
                    "{:?}",
                    blocks
                        .iter()
                        .flat_map(|block| block.inline())
                        .collect::<Vec<_>>()
                );
                let figures: Vec<_> = blocks.iter().filter_map(|block| block.figure()).collect();
                if ALL_KINDS.iter().all(|kind| found.contains(kind))
                    && callouts.len() == 3
                    && ["Strong", "Emphasis", "Code", "Link", "LineBreak"]
                        .iter()
                        .all(|name| inline.contains(name))
                    && figures.iter().any(|f| matches!(f, FigureSource::Svg(_)))
                    && figures
                        .iter()
                        .any(|f| matches!(f, FigureSource::Mermaid(_)))
                    && blocks.iter().all(|block| !block.is_new())
                {
                    return;
                }
                failures.push("it parses but does not use every block type, callout kind and inline markup, an SVG and a Mermaid figure, and a data-node-id on every block".to_string());
            }
        }
    }
    panic!(
        "one ```html example in docs/formats/html-docs.md must be a complete, valid doc that uses every block type, callout kind and inline markup, an SVG and a Mermaid figure, with a data-node-id on every block; the examples fail with: {failures:?}"
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
