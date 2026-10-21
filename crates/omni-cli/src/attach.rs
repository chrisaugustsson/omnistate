//! `omni attach <file>`: uploads a file once, so that docs, designs, fonts and
//! posts can refer to it as `attachment:<id>`. The file's bytes are the body of
//! the request; its base name travels percent-encoded in `X-Omni-Filename`.

use crate::args::Args;
use crate::commands::{Agent, agent, say};
use crate::error::{CliError, Result, bail};
use crate::http::Response;
use crate::json::Value;
use std::path::Path;

/// The largest file the API takes (20 MiB), checked here before any bytes are sent.
const MAX_BYTES: u64 = 20 * 1024 * 1024;

pub fn run(args: &Args, file: &str) -> Result<()> {
    let path = Path::new(file);
    let metadata = std::fs::metadata(path)
        .map_err(|e| CliError::new(format!("cannot read {file}: {e}; check the path")))?;
    if metadata.is_dir() {
        bail!("{file} is a directory; `omni attach` uploads one file: omni attach <file>");
    }
    if metadata.len() > MAX_BYTES {
        bail!("{file} is larger than 20 MB, the limit for an attachment; attach a smaller file");
    }
    let bytes = std::fs::read(path)
        .map_err(|e| CliError::new(format!("cannot read {file}: {e}; check the path")))?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| file.to_string());

    let agent = agent(args)?;
    let response = send(&agent, &name, &bytes)?;
    let text = response.text();
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    let body = Value::parse(&text).map_err(|_| {
        CliError::new(
            "the server's answer about the attachment is not readable; is it an OmniState API?",
        )
    })?;
    let attachment = body.get("attachment").unwrap_or(&body);
    match (attachment.str_at("ref"), attachment.str_at("id")) {
        (Some(reference), _) => say(reference),
        (None, Some(id)) => say(&format!("attachment:{id}")),
        (None, None) => bail!("the server's answer has no attachment ID; try again"),
    }
    Ok(())
}

/// Uploads `bytes` as a file named `name`.
pub(crate) fn send(agent: &Agent, name: &str, bytes: &[u8]) -> Result<Response> {
    let headers = [
        (
            "Content-Type".to_string(),
            "application/octet-stream".to_string(),
        ),
        ("X-Omni-Filename".to_string(), percent_encode(name)),
    ];
    agent.request_with(
        "POST",
        &format!("/api/v1/w/{}/attachments", agent.workspace_id),
        &headers,
        Some(bytes),
    )
}

/// The largest file the API takes, in bytes.
pub(crate) const MAX_UPLOAD: usize = MAX_BYTES as usize;

/// The text with every byte but the unreserved characters as `%XX`, so a name
/// with a space, a line break or a letter outside ASCII fits in a header.
fn percent_encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_encoded_for_a_header() {
        assert_eq!(percent_encode("logo.png"), "logo.png");
        assert_eq!(
            percent_encode("my logo \u{e9}.png"),
            "my%20logo%20%C3%A9.png"
        );
        assert_eq!(percent_encode("a\r\nb"), "a%0D%0Ab");
    }
}
