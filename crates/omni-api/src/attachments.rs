//! Attachments: files that agents upload once and refer to everywhere as
//! `attachment:<id>`. The bytes live in RustFS at
//! `ws/{workspace_id}/attachments/{id}`; the metadata is a row of the tenant
//! table `attachments`. What a file is comes from its bytes alone: the name and
//! the type the sender claims are never trusted, so a page of script named
//! `logo.png` is a plain download. Only a PNG, JPEG, GIF or WebP whose bytes
//! confirm it is ever served inline; everything else is a download.

use crate::AppState;
use crate::session::{ApiError, ErrorBody};
use crate::workspace::{AgentRequest, WorkspaceRequest, parse_uuid};
use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{Path, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use utoipa::ToSchema;

/// The largest file, in bytes. The `omni` command line says "20 MB".
pub const MAX_BYTES: usize = 20 * 1024 * 1024;

/// The header that carries the file's name, percent-encoded UTF-8.
const FILENAME_HEADER: &str = "x-omni-filename";
const MAX_FILENAME_CHARS: usize = 255;

#[derive(Serialize, ToSchema)]
pub struct AttachmentBody {
    pub id: String,
    /// How docs, designs and posts refer to the file: `attachment:<id>`.
    #[serde(rename = "ref")]
    pub reference: String,
    pub filename: String,
    /// `image` (PNG, JPEG, WebP, GIF), `font` (WOFF2, TTF, OTF) or `file`
    /// (anything else, download only).
    pub kind: String,
    /// Decided from the file's bytes: `image/*`, `font/*` or
    /// `application/octet-stream`.
    pub content_type: String,
    pub size: i64,
    pub created_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct AttachmentResponse {
    pub attachment: AttachmentBody,
}

/// What the bytes of a file are.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Sniffed {
    Image(&'static str),
    Font(&'static str),
    Other,
}

impl Sniffed {
    fn of(bytes: &[u8]) -> Self {
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            Self::Image("image/png")
        } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
            Self::Image("image/jpeg")
        } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            Self::Image("image/gif")
        } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            Self::Image("image/webp")
        } else if bytes.starts_with(b"wOF2") {
            Self::Font("font/woff2")
        } else if bytes.starts_with(&[0x00, 0x01, 0x00, 0x00]) {
            Self::Font("font/ttf")
        } else if bytes.starts_with(b"OTTO") {
            Self::Font("font/otf")
        } else {
            Self::Other
        }
    }

    fn content_type(self) -> &'static str {
        match self {
            Self::Image(content_type) | Self::Font(content_type) => content_type,
            Self::Other => "application/octet-stream",
        }
    }

    fn kind(self) -> &'static str {
        match self {
            Self::Image(_) => "image",
            Self::Font(_) => "font",
            Self::Other => "file",
        }
    }
}

fn storage_key(workspace_id: &str, id: &str) -> String {
    format!("ws/{workspace_id}/attachments/{id}")
}

/// The text of a percent-encoded value; a `%` that is not followed by two hex
/// digits stays as it is, and bytes that are not UTF-8 become U+FFFD.
fn percent_decode(text: &[u8]) -> String {
    let mut out = Vec::with_capacity(text.len());
    let mut at = 0;
    while at < text.len() {
        let hex = text
            .get(at + 1..at + 3)
            .and_then(|pair| std::str::from_utf8(pair).ok())
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        match (text[at], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                at += 3;
            }
            (byte, _) => {
                out.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The name from `X-Omni-Filename`: no control characters (they would let a
/// name add lines to a header), no longer than 255 characters, never empty.
fn filename_of(headers: &HeaderMap) -> String {
    let decoded = headers
        .get(FILENAME_HEADER)
        .map(|value| percent_decode(value.as_bytes()))
        .unwrap_or_default();
    let cleaned: String = decoded
        .chars()
        .map(|c| if c.is_control() { '_' } else { c })
        .take(MAX_FILENAME_CHARS)
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        "attachment".to_string()
    } else {
        cleaned.to_string()
    }
}

/// A `Content-Disposition: attachment` value that names the file: a plain ASCII
/// name for old clients and the real one as `filename*`.
fn download_disposition(filename: &str) -> String {
    let plain: String = filename
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                if matches!(c, '"' | '\\' | '/' | '%' | ';') {
                    '_'
                } else {
                    c
                }
            } else {
                '_'
            }
        })
        .collect();
    let mut encoded = String::new();
    for byte in filename.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    format!("attachment; filename=\"{plain}\"; filename*=UTF-8''{encoded}")
}

/// Uploads a file. The body is the file's bytes; `X-Omni-Filename` is its name.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/attachments",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("X-Omni-Filename" = Option<String>, Header, description = "The file's name, percent-encoded UTF-8"),
    ),
    request_body(content = Vec<u8>, content_type = "application/octet-stream", description = "The file's bytes, at most 20 MB"),
    responses(
        (status = 201, description = "The stored attachment", body = AttachmentResponse),
        (status = 400, description = "The body could not be read", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "The caller cannot reach this workspace", body = ErrorBody),
        (status = 413, description = "The file is over 20 MB", body = ErrorBody),
    )
)]
pub async fn upload(
    State(state): State<AppState>,
    request: AgentRequest,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<(StatusCode, Json<AttachmentResponse>), ApiError> {
    let body = body.map_err(|rejection| {
        if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            ApiError::Refused(StatusCode::PAYLOAD_TOO_LARGE, "file_too_large")
        } else {
            ApiError::bad_request("invalid_request")
        }
    })?;
    if body.len() > MAX_BYTES {
        return Err(ApiError::Refused(
            StatusCode::PAYLOAD_TOO_LARGE,
            "file_too_large",
        ));
    }
    let sniffed = Sniffed::of(&body);
    let filename = filename_of(&headers);

    // The row goes in first and the file second, in one transaction: a file
    // that cannot be stored leaves no row behind.
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let (id, created_at): (String, String) = sqlx::query_as(concat!(
        "INSERT INTO attachments (workspace_id, filename, content_type, size_bytes) \
         VALUES ($1::uuid, $2, $3, $4) RETURNING id::text, ",
        "to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')"
    ))
    .bind(&request.workspace_id)
    .bind(&filename)
    .bind(sniffed.content_type())
    .bind(body.len() as i64)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    state
        .bucket
        .put_object(storage_key(&request.workspace_id, &id), &body)
        .await
        .map_err(|e| {
            eprintln!("omni-api: cannot store attachment {id}: {e}");
            ApiError::Unavailable
        })?;
    tx.commit().await.map_err(ApiError::database)?;

    Ok((
        StatusCode::CREATED,
        Json(AttachmentResponse {
            attachment: AttachmentBody {
                reference: format!("attachment:{id}"),
                id,
                filename,
                kind: sniffed.kind().to_string(),
                content_type: sniffed.content_type().to_string(),
                size: body.len() as i64,
                created_at,
            },
        }),
    ))
}

/// The bytes of a file. An image whose bytes confirm its type is served inline;
/// anything else is a download.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/attachments/{attachment_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("attachment_id" = String, Path, description = "The attachment's UUID"),
    ),
    responses(
        (status = 200, description = "The file's bytes", content_type = "application/octet-stream"),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "The caller cannot reach this workspace, or it has no such attachment", body = ErrorBody),
    )
)]
pub async fn download(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, attachment_id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let id = parse_uuid(&attachment_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let filename: String =
        sqlx::query_scalar("SELECT filename FROM attachments WHERE id = $1::uuid")
            .bind(&id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(ApiError::database)?
            .ok_or(ApiError::NotFound)?;
    tx.commit().await.map_err(ApiError::database)?;

    let object = state
        .bucket
        .get_object(storage_key(&request.workspace_id, &id))
        .await
        .map_err(|e| {
            eprintln!("omni-api: cannot read attachment {id}: {e}");
            ApiError::Unavailable
        })?;
    match object.status_code() {
        200 => {}
        404 => return Err(ApiError::NotFound),
        _ => return Err(ApiError::Unavailable),
    }
    let bytes = object.to_vec();

    // The decision is made from the bytes that are served now, not from the
    // type the row records.
    let sniffed = Sniffed::of(&bytes);
    let mut response = bytes.into_response();
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static(sniffed.content_type()),
    );
    let disposition = match sniffed {
        Sniffed::Image(_) => HeaderValue::from_static("inline"),
        Sniffed::Font(_) | Sniffed::Other => {
            HeaderValue::from_str(&download_disposition(&filename))
                .unwrap_or_else(|_| HeaderValue::from_static("attachment"))
        }
    };
    headers.insert(CONTENT_DISPOSITION, disposition);
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bytes_decide_what_a_file_is() {
        let cases: [(&[u8], &str, &str); 13] = [
            (b"\x89PNG\r\n\x1a\n....", "image", "image/png"),
            (&[0xff, 0xd8, 0xff, 0xe0], "image", "image/jpeg"),
            (b"GIF87a....", "image", "image/gif"),
            (b"GIF89a....", "image", "image/gif"),
            (b"RIFF\0\0\0\0WEBPVP8 ", "image", "image/webp"),
            (b"RIFF\0\0\0\0WAVEfmt ", "file", "application/octet-stream"),
            (b"<svg><script/></svg>", "file", "application/octet-stream"),
            (
                b"<html><script/></html>",
                "file",
                "application/octet-stream",
            ),
            (b"not a font", "file", "application/octet-stream"),
            (b"", "file", "application/octet-stream"),
            (b"wOF2....", "font", "font/woff2"),
            (&[0x00, 0x01, 0x00, 0x00], "font", "font/ttf"),
            (b"OTTO....", "font", "font/otf"),
        ];
        for (bytes, kind, content_type) in cases {
            let sniffed = Sniffed::of(bytes);
            assert_eq!(
                (sniffed.kind(), sniffed.content_type()),
                (kind, content_type),
                "{bytes:?}"
            );
        }
    }

    #[test]
    fn a_name_cannot_carry_a_line_break_into_a_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            FILENAME_HEADER,
            HeaderValue::from_static("evil%0D%0Ax-injected:%201.txt"),
        );
        let name = filename_of(&headers);
        assert!(!name.contains(['\r', '\n']), "{name:?}");
        let disposition = download_disposition("a\"b/c\u{e5}.txt");
        assert!(HeaderValue::from_str(&disposition).is_ok());
        assert!(
            disposition.to_ascii_lowercase().starts_with("attachment;"),
            "{disposition}"
        );
        assert_eq!(disposition.matches('"').count(), 2, "{disposition}");
        assert!(
            disposition.ends_with("filename*=UTF-8''a%22b%2Fc%C3%A5.txt"),
            "{disposition}"
        );
    }
}
