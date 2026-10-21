//! US-034: post bodies are Markdown, rendered to HTML by
//! `omni_core::markdown::render`, which compiles natively and to WebAssembly.
//!
//! The contract these tests pin down (the rest is the green step's choice):
//!
//! - CommonMark renders as usual: headings, paragraphs, strong and emphasis,
//!   code spans and code blocks, block quotes and lists.
//! - Raw HTML is switched off: HTML in the Markdown, as a block or inline, never
//!   becomes a tag of the output. It may be shown as escaped text or left out.
//!   Every `<` of the output starts a tag (or an HTML comment), so text is
//!   always escaped.
//! - The output only has the tags that Markdown makes (`p`, `h1` to `h6`,
//!   `strong`, `em`, `del`, `s`, `code`, `pre`, `blockquote`, `ul`, `ol`, `li`,
//!   `a`, `img`, `hr`, `br`, the table tags and the task list `input`), each
//!   with the attributes listed in `ALLOWED` and no others.
//! - A link keeps its target only if it uses http, https or mailto; any other
//!   target (javascript:, data:, vbscript:, ftp:, relative or
//!   protocol-relative addresses, `attachment:`), however it is written, gives
//!   no `href`. The link's text stays.
//! - An image is only drawn when its source is `attachment:<id>` with a UUID:
//!   `<img src="attachment:<id>" alt="...">`. Any other source draws no
//!   image.

use omni_core::markdown::render;

const PHOTO_ID: &str = "0192f1c4-5b6a-7d8e-9f01-23456789abcd";

/// The tags the output may have, each with the attributes it may carry.
const ALLOWED: &[(&str, &[&str])] = &[
    ("p", &[]),
    ("h1", &[]),
    ("h2", &[]),
    ("h3", &[]),
    ("h4", &[]),
    ("h5", &[]),
    ("h6", &[]),
    ("strong", &[]),
    ("em", &[]),
    ("del", &[]),
    ("s", &[]),
    ("code", &["class"]),
    ("pre", &[]),
    ("blockquote", &[]),
    ("ul", &[]),
    ("ol", &["start"]),
    ("li", &[]),
    ("a", &["href", "title"]),
    ("img", &["src", "alt", "title"]),
    ("hr", &[]),
    ("br", &[]),
    ("table", &[]),
    ("thead", &[]),
    ("tbody", &[]),
    ("tr", &[]),
    ("th", &["align", "style"]),
    ("td", &["align", "style"]),
    ("input", &["type", "checked", "disabled"]),
];

/// One tag of the output.
#[derive(Debug)]
struct Tag {
    name: String,
    closing: bool,
    attributes: Vec<(String, String)>,
}

impl Tag {
    fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// The text of an attribute value with the usual entities decoded.
fn decode(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// The tags of `html`, in order. Panics on a `<` that starts no tag, which
/// would be text that was not escaped.
fn tags(html: &str) -> Vec<Tag> {
    let chars: Vec<char> = html.chars().collect();
    let mut tags = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        if chars[at] != '<' {
            at += 1;
            continue;
        }
        let rest: String = chars[at..].iter().collect();
        if rest.starts_with("<!--") {
            let end = rest
                .find("-->")
                .unwrap_or_else(|| panic!("an unclosed comment in {html:?}"));
            at += rest[..end + 3].chars().count();
            continue;
        }
        at += 1;
        let closing = chars.get(at) == Some(&'/');
        if closing {
            at += 1;
        }
        let start = at;
        while at < chars.len() && chars[at].is_ascii_alphanumeric() {
            at += 1;
        }
        let name: String = chars[start..at].iter().collect::<String>().to_lowercase();
        assert!(
            !name.is_empty() && chars[start].is_ascii_alphabetic(),
            "a `<` that starts no tag, so text was not escaped: {html:?}"
        );
        let mut attributes = Vec::new();
        loop {
            while at < chars.len() && chars[at].is_whitespace() {
                at += 1;
            }
            match chars.get(at) {
                None => panic!("an unclosed tag <{name} in {html:?}"),
                Some('>') => {
                    at += 1;
                    break;
                }
                Some('/') => {
                    at += 1;
                    continue;
                }
                _ => {}
            }
            let start = at;
            while at < chars.len()
                && !matches!(chars[at], '=' | '>' | '/')
                && !chars[at].is_whitespace()
            {
                at += 1;
            }
            let attribute: String = chars[start..at].iter().collect::<String>().to_lowercase();
            let mut value = String::new();
            if chars.get(at) == Some(&'=') {
                at += 1;
                match chars.get(at) {
                    Some(quote @ ('"' | '\'')) => {
                        let quote = *quote;
                        at += 1;
                        let start = at;
                        while at < chars.len() && chars[at] != quote {
                            at += 1;
                        }
                        value = chars[start..at].iter().collect();
                        at += 1;
                    }
                    _ => {
                        let start = at;
                        while at < chars.len() && chars[at] != '>' && !chars[at].is_whitespace() {
                            at += 1;
                        }
                        value = chars[start..at].iter().collect();
                    }
                }
            }
            attributes.push((attribute, decode(&value)));
        }
        tags.push(Tag {
            name,
            closing,
            attributes,
        });
    }
    tags
}

fn is_uuid(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => *b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
}

fn allowed_href(href: &str) -> bool {
    let lower = href.to_ascii_lowercase();
    ["http://", "https://", "mailto:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme) && lower.len() > scheme.len())
}

fn allowed_src(src: &str) -> bool {
    src.strip_prefix("attachment:").is_some_and(is_uuid)
}

/// Renders `markdown` and checks that the output keeps to the allow-lists;
/// returns the output and its tags.
fn safe(markdown: &str) -> (String, Vec<Tag>) {
    let html = render(markdown);
    let found = tags(&html);
    for tag in &found {
        let allowed = ALLOWED
            .iter()
            .find(|(name, _)| *name == tag.name)
            .unwrap_or_else(|| {
                panic!(
                    "the tag <{}> is not one that Markdown makes: {markdown:?} rendered as {html:?}",
                    tag.name
                )
            })
            .1;
        if tag.closing {
            continue;
        }
        for (attribute, value) in &tag.attributes {
            assert!(
                allowed.contains(&attribute.as_str()),
                "<{}> may not carry `{attribute}`: {markdown:?} rendered as {html:?}",
                tag.name
            );
            if tag.name == "a" && attribute == "href" {
                assert!(
                    allowed_href(value),
                    "a link to {value:?} is not http, https or mailto: {markdown:?} rendered as {html:?}"
                );
            }
            if tag.name == "img" && attribute == "src" {
                assert!(
                    allowed_src(value),
                    "an image from {value:?} is not an attachment: {markdown:?} rendered as {html:?}"
                );
            }
            if attribute == "style" {
                assert!(
                    value.trim_start().starts_with("text-align"),
                    "a style other than text-align: {markdown:?} rendered as {html:?}"
                );
            }
        }
        if tag.name == "img" {
            assert!(
                tag.attribute("src").is_some(),
                "an image without a source: {markdown:?} rendered as {html:?}"
            );
        }
    }
    (html, found)
}

fn opening<'a>(tags: &'a [Tag], name: &str) -> Vec<&'a Tag> {
    tags.iter()
        .filter(|tag| tag.name == name && !tag.closing)
        .collect()
}

// ---------------------------------------------------------------------------
// Markdown
// ---------------------------------------------------------------------------

#[test]
fn common_markdown_renders_as_html() {
    let markdown = "# The hero\n\n\
                    Some **bold** and *emphasis* with `code`.\n\n\
                    > Quoted\n\n\
                    - one\n\
                    - two\n\n\
                    1. first\n\
                    2. second\n\n\
                    ```\nlet x = 1;\n```\n";
    let (html, found) = safe(markdown);
    for needle in [
        "<h1>The hero</h1>",
        "<strong>bold</strong>",
        "<em>emphasis</em>",
        "<code>code</code>",
        "<li>one</li>",
        "<li>two</li>",
        "<li>first</li>",
        "let x = 1;",
    ] {
        assert!(html.contains(needle), "{needle:?} is in {html:?}");
    }
    for name in ["blockquote", "ul", "ol", "pre", "p"] {
        assert!(
            !opening(&found, name).is_empty(),
            "the output has a <{name}>: {html:?}"
        );
    }
}

#[test]
fn text_is_escaped() {
    let (html, _) = safe("5 < 6 & 7 > 3\n");
    assert!(
        html.contains("5 &lt; 6 &amp; 7 &gt; 3"),
        "the text is escaped: {html:?}"
    );
}

// ---------------------------------------------------------------------------
// Raw HTML
// ---------------------------------------------------------------------------

#[test]
fn raw_html_never_becomes_a_tag() {
    for markdown in [
        "<script>alert(1)</script>\n",
        "Hello <script>alert(1)</script> there\n",
        "<div onclick=\"steal()\">\n\nhi\n\n</div>\n",
        "<p onclick=\"steal()\">a paragraph</p>\n",
        "Some <b onmouseover=\"steal()\">bold</b> text\n",
        "<img src=x onerror=alert(1)>\n",
        "An inline <img src=x onerror=alert(1)> image\n",
        "<iframe src=\"https://evil.example\"></iframe>\n",
        "<style>body { display: none }</style>\n",
        "<a href=\"javascript:alert(1)\">click</a>\n",
        "<svg onload=\"alert(1)\"></svg>\n",
        "<SCRIPT>alert(1)</SCRIPT>\n",
        "<form action=\"https://evil.example\"><input name=\"x\"></form>\n",
    ] {
        let (html, found) = safe(markdown);
        let lower = html.to_ascii_lowercase();
        for tag in [
            "<script", "<div", "<b ", "<iframe", "<style", "<svg", "<form",
        ] {
            assert!(
                !lower.contains(tag),
                "{markdown:?} rendered raw HTML ({tag}): {html:?}"
            );
        }
        assert!(
            opening(&found, "img").is_empty() && opening(&found, "a").is_empty(),
            "{markdown:?} made an image or a link out of raw HTML: {html:?}"
        );
        assert!(
            found.iter().all(|tag| tag
                .attributes
                .iter()
                .all(|(name, _)| !name.starts_with("on"))),
            "{markdown:?} rendered an event handler: {html:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Links
// ---------------------------------------------------------------------------

#[test]
fn links_keep_http_https_and_mailto_targets() {
    for (markdown, href, text) in [
        (
            "See [the brief](https://example.com/brief?a=1&b=2).\n",
            "https://example.com/brief?a=1&b=2",
            "the brief",
        ),
        (
            "See [the old site](http://example.com/).\n",
            "http://example.com/",
            "the old site",
        ),
        (
            "Ask [Ann](mailto:ann@example.com).\n",
            "mailto:ann@example.com",
            "Ann",
        ),
        (
            "Autolinked <https://example.com/x>.\n",
            "https://example.com/x",
            "https://example.com/x",
        ),
        (
            "By reference [the spec][spec].\n\n[spec]: https://example.com/spec\n",
            "https://example.com/spec",
            "the spec",
        ),
    ] {
        let (html, found) = safe(markdown);
        let links = opening(&found, "a");
        assert_eq!(links.len(), 1, "{markdown:?} has one link: {html:?}");
        assert_eq!(
            links[0].attribute("href"),
            Some(href),
            "{markdown:?} keeps its target: {html:?}"
        );
        assert!(html.contains(text), "the link's text stays: {html:?}");
    }
}

#[test]
fn links_with_any_other_target_lose_it_and_keep_their_text() {
    for markdown in [
        "[click](javascript:alert(1))\n",
        "[click](JavaScript:alert(1))\n",
        "[click](  javascript:alert(1)  )\n",
        "[click](java&#115;cript:alert(1))\n",
        "[click](<javascript:alert(1)>)\n",
        "[click](data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==)\n",
        "[click](vbscript:msgbox(1))\n",
        "[click](ftp://example.com/file)\n",
        "[click](/api/v1/w/x/attachments/y)\n",
        "[click](//evil.example/path)\n",
        "[click](attachment:0192f1c4-5b6a-7d8e-9f01-23456789abcd)\n",
        "[click][evil]\n\n[evil]: javascript:alert(1)\n",
        "<javascript:alert(1)>\n",
        "[click](mailto:)\n",
    ] {
        let (html, found) = safe(markdown);
        assert!(
            opening(&found, "a")
                .iter()
                .all(|link| link.attribute("href").is_none()),
            "{markdown:?} kept a target: {html:?}"
        );
        assert!(
            !html.to_ascii_lowercase().contains("href"),
            "{markdown:?} rendered an href: {html:?}"
        );
        if markdown.starts_with("[click]") {
            assert!(html.contains("click"), "the link's text stays: {html:?}");
        }
    }
}

// ---------------------------------------------------------------------------
// Images
// ---------------------------------------------------------------------------

#[test]
fn an_image_from_an_attachment_is_drawn() {
    let markdown = format!("Look: ![A sketch of the hero](attachment:{PHOTO_ID})\n");
    let (html, found) = safe(&markdown);
    let images = opening(&found, "img");
    assert_eq!(images.len(), 1, "one image: {html:?}");
    assert_eq!(
        images[0].attribute("src"),
        Some(format!("attachment:{PHOTO_ID}").as_str()),
        "{html:?}"
    );
    assert_eq!(
        images[0].attribute("alt"),
        Some("A sketch of the hero"),
        "{html:?}"
    );
}

#[test]
fn an_image_from_anywhere_else_is_not_drawn() {
    for markdown in [
        "![x](https://example.com/x.png)\n",
        "![x](http://example.com/x.png)\n",
        "![x](data:image/png;base64,iVBORw0KGgo=)\n",
        "![x](javascript:alert(1))\n",
        "![x](mailto:ann@example.com)\n",
        "![x](attachment:not-an-id)\n",
        "![x](attachment:)\n",
        "![x](/api/v1/w/x/attachments/0192f1c4-5b6a-7d8e-9f01-23456789abcd)\n",
        "![x](//evil.example/x.png)\n",
        "![x][pic]\n\n[pic]: https://example.com/x.png\n",
        "<img src=\"attachment:0192f1c4-5b6a-7d8e-9f01-23456789abcd\" onerror=\"alert(1)\">\n",
    ] {
        let (html, found) = safe(markdown);
        assert!(
            opening(&found, "img").is_empty(),
            "{markdown:?} drew an image: {html:?}"
        );
    }
}
