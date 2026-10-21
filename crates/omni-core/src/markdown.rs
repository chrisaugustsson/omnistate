//! Post bodies (US-034) are Markdown. They are rendered to HTML with raw HTML
//! switched off, links restricted to http, https and mailto, and images
//! restricted to workspace attachments, written `attachment:<id>`.

/// Renders a post body to HTML that is safe to show.
pub fn render(markdown: &str) -> String {
    let _ = markdown;
    todo!("US-034: render a post body")
}
