//! Post bodies (US-034) are Markdown. They are rendered to HTML with raw HTML
//! switched off, links restricted to http, https and mailto, and images
//! restricted to workspace attachments, written `attachment:<id>`.

use pulldown_cmark::{CowStr, Event, LinkType, Options, Parser, Tag, TagEnd, html};

/// Renders a post body to HTML that is safe to show.
///
/// Raw HTML in the Markdown, as a block or inline, is shown as text and never
/// becomes a tag. A link keeps its target only when it is an http, https or
/// mailto address; any other link loses its target and is left as its text. An
/// image is drawn only when its source is `attachment:<id>` with a UUID; any
/// other image is left as its alt text.
pub fn render(markdown: &str) -> String {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    // For each link or image that is open: whether its tag is written.
    let mut open: Vec<bool> = Vec::new();
    let mut images_open = 0;
    let events = Parser::new_ext(markdown, options).filter_map(|event| {
        Some(match event {
            Event::Html(text) | Event::InlineHtml(text) => Event::Text(text),
            Event::Start(Tag::HtmlBlock) => Event::Start(Tag::Paragraph),
            Event::End(TagEnd::HtmlBlock) => Event::End(TagEnd::Paragraph),
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                ..
            }) if !allowed_link(link_type, &dest_url) => {
                open.push(false);
                return None;
            }
            Event::Start(Tag::Image { ref dest_url, .. })
                if images_open > 0 || !allowed_image(dest_url) =>
            {
                open.push(false);
                images_open += 1;
                return None;
            }
            Event::Start(tag @ Tag::Link { .. }) => {
                open.push(true);
                Event::Start(tag)
            }
            Event::Start(tag @ Tag::Image { .. }) => {
                open.push(true);
                images_open += 1;
                Event::Start(tag)
            }
            Event::End(end @ (TagEnd::Link | TagEnd::Image)) => {
                if end == TagEnd::Image {
                    images_open -= 1;
                }
                if !open.pop().unwrap_or(true) {
                    return None;
                }
                Event::End(end)
            }
            event => event,
        })
    });
    let mut out = String::new();
    html::push_html(&mut out, events);
    out
}

/// Whether a link to `dest` keeps its target.
fn allowed_link(link_type: LinkType, dest: &CowStr) -> bool {
    if dest.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return false;
    }
    // The destination of an e-mail address in angle brackets is the address
    // without its `mailto:`, which the HTML writer adds.
    if link_type == LinkType::Email {
        return !dest.is_empty();
    }
    let lower = dest.to_ascii_lowercase();
    ["http://", "https://", "mailto:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme) && lower.len() > scheme.len())
}

/// Whether an image from `dest` is drawn: only `attachment:<uuid>`.
fn allowed_image(dest: &CowStr) -> bool {
    dest.strip_prefix("attachment:").is_some_and(is_uuid)
}

/// A UUID in its 36-character form.
fn is_uuid(text: &str) -> bool {
    text.len() == 36
        && text.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
}
