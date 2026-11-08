//! An HTTP/1.1 client over `std::net`: one request per connection, with
//! `Connection: close`. An `http://` address is plain TCP; an `https://` one
//! wraps the connection in rustls, which checks the certificate against the
//! operating system's roots, plus the certificates in `OMNI_CA_FILE`.

use crate::error::{CliError, Result, bail};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const IO_TIMEOUT: Duration = Duration::from_secs(60);

/// Where the API is: the host and port, and a path prefix if the proxy has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    host: String,
    port: u16,
    base: String,
    tls: bool,
}

impl Server {
    pub fn parse(text: &str) -> Result<Server> {
        let text = text.trim();
        let (tls, rest) = if let Some(rest) = text.strip_prefix("https://") {
            (true, rest)
        } else if let Some(rest) = text.strip_prefix("http://") {
            (false, rest)
        } else {
            bail!(
                "{text:?} is not a server address; use one like http://omnistate.localhost or https://omnistate.example.com (--server or OMNI_SERVER)"
            );
        };
        let (authority, base) = match rest.find('/') {
            Some(at) => (&rest[..at], rest[at..].trim_end_matches('/')),
            None => (rest, ""),
        };
        let (host, port) = if let Some(inner) = authority.strip_prefix('[') {
            let (host, after) = inner
                .split_once(']')
                .ok_or_else(|| CliError::new(format!("{text:?} has an unclosed [ in the host")))?;
            (host, after.strip_prefix(':'))
        } else {
            match authority.rsplit_once(':') {
                Some((host, port)) => (host, Some(port)),
                None => (authority, None),
            }
        };
        let port = match port {
            None if tls => 443,
            None => 80,
            Some(port) => port
                .parse()
                .map_err(|_| CliError::new(format!("{text:?} has an invalid port")))?,
        };
        if host.is_empty() {
            bail!(
                "{text:?} has no host; use an address like http://omnistate.localhost or https://omnistate.example.com"
            );
        }
        Ok(Server {
            host: host.to_string(),
            port,
            base: base.to_string(),
            tls,
        })
    }

    /// `host:port`, as the error messages and the Host header name it.
    pub fn address(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    /// The values of every header called `name`, in the order they came.
    pub fn headers_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> {
        self.headers
            .iter()
            .filter(move |(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Sends one request and reads the whole response.
pub fn send(
    server: &Server,
    method: &str,
    path: &str,
    headers: &[(String, String)],
    body: Option<&[u8]>,
) -> Result<Response> {
    let bad_text = |text: &str| text.chars().any(|c| c.is_control() || c == ' ');
    if !path.starts_with('/') || bad_text(path) {
        bail!("{path:?} is not a request path; it starts with / and has no spaces");
    }
    if headers
        .iter()
        .any(|(_, value)| value.contains(['\r', '\n']))
    {
        bail!("a header value has a line break in it");
    }

    let address = server.address();
    let mut stream = connect(server, &address)?;
    let mut head = format!(
        "{method} {}{path} HTTP/1.1\r\nHost: {address}\r\nUser-Agent: omni/{}\r\nAccept: application/json\r\nConnection: close\r\n",
        server.base,
        env!("CARGO_PKG_VERSION")
    );
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    if body.is_some() || !matches!(method, "GET" | "HEAD") {
        head.push_str(&format!(
            "Content-Length: {}\r\n",
            body.map_or(0, <[u8]>::len)
        ));
    }
    head.push_str("\r\n");

    let io_error = |e: std::io::Error| {
        if matches!(
            e.kind(),
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
        ) {
            CliError::new(format!("the server at {address} did not answer in time"))
        } else {
            CliError::new(format!(
                "the connection to {address} broke ({e}); try again, or check the server with --server or OMNI_SERVER"
            ))
        }
    };
    stream.write_all(head.as_bytes()).map_err(io_error)?;
    if let Some(body) = body {
        stream.write_all(body).map_err(io_error)?;
    }
    stream.flush().map_err(io_error)?;

    let mut raw = Vec::new();
    if let Err(e) = stream.read_to_end(&mut raw) {
        // A TLS server may close without a close_notify once it has sent everything.
        if !(server.tls && e.kind() == std::io::ErrorKind::UnexpectedEof && !raw.is_empty()) {
            return Err(io_error(e));
        }
    }
    parse_response(raw, method == "HEAD").ok_or_else(|| {
        CliError::new(format!(
            "the server at {address} sent something that is not an HTTP response; is --server or OMNI_SERVER right?"
        ))
    })
}

trait Stream: Read + Write {}
impl<T: Read + Write> Stream for T {}

fn connect(server: &Server, address: &str) -> Result<Box<dyn Stream>> {
    let unreachable = |reason: String| {
        CliError::new(format!(
            "cannot reach the server at {address} ({reason}); check that the stack is running and that --server or OMNI_SERVER is right (an http:// or https:// address)"
        ))
    };
    let tls_config = if server.tls {
        Some(tls_config()?)
    } else {
        None
    };
    let addrs = (server.host.as_str(), server.port)
        .to_socket_addrs()
        .map_err(|e| unreachable(e.to_string()))?;
    let mut last = "no address".to_string();
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(stream) => {
                let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
                let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
                let _ = stream.set_nodelay(true);
                let Some(config) = tls_config else {
                    return Ok(Box::new(stream));
                };
                return tls_handshake(server, address, config, stream);
            }
            Err(e) => last = e.to_string(),
        }
    }
    Err(unreachable(last))
}

/// The client configuration: the operating system's roots and `OMNI_CA_FILE`'s
/// certificates. There is no way to switch the check off.
fn tls_config() -> Result<Arc<rustls::ClientConfig>> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add_parsable_certificates(rustls_native_certs::load_native_certs().certs);
    if let Some(path) = std::env::var_os("OMNI_CA_FILE").filter(|path| !path.is_empty()) {
        let shown = path.to_string_lossy().into_owned();
        let certs = CertificateDer::pem_file_iter(&path)
            .and_then(|certs| certs.collect::<std::result::Result<Vec<_>, _>>())
            .map_err(|e| {
                CliError::new(format!(
                    "OMNI_CA_FILE {shown:?} could not be read as a PEM file of certificates ({e}); fix the path or unset OMNI_CA_FILE"
                ))
            })?;
        if certs.is_empty() {
            bail!(
                "OMNI_CA_FILE {shown:?} holds no certificate; fix the path or unset OMNI_CA_FILE"
            );
        }
        let (_, rejected) = roots.add_parsable_certificates(certs);
        if rejected > 0 {
            bail!(
                "OMNI_CA_FILE {shown:?} holds a certificate that is not valid; fix the file or unset OMNI_CA_FILE"
            );
        }
    }
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| CliError::new(format!("cannot set up TLS ({e})")))?
    .with_root_certificates(roots)
    .with_no_client_auth();
    Ok(Arc::new(config))
}

/// Runs the TLS handshake, so a certificate that fails the check is refused
/// before any request is sent.
fn tls_handshake(
    server: &Server,
    address: &str,
    config: Arc<rustls::ClientConfig>,
    mut tcp: TcpStream,
) -> Result<Box<dyn Stream>> {
    let name = ServerName::try_from(server.host.clone())
        .map_err(|_| CliError::new(format!("{:?} is not a valid host name", server.host)))?;
    let mut conn = rustls::ClientConnection::new(config, name)
        .map_err(|e| CliError::new(format!("cannot set up TLS to {address} ({e})")))?;
    while conn.is_handshaking() {
        if let Err(e) = conn.complete_io(&mut tcp) {
            let certificate = e
                .get_ref()
                .and_then(|inner| inner.downcast_ref::<rustls::Error>())
                .is_some_and(|inner| matches!(inner, rustls::Error::InvalidCertificate(_)));
            return Err(if certificate {
                CliError::new(format!(
                    "the certificate of the server at {address} could not be verified ({e}); nothing was sent. For a server with its own certificate authority, set OMNI_CA_FILE to a PEM file with its certificate"
                ))
            } else {
                CliError::new(format!(
                    "cannot reach the server at {address} over TLS ({e}); check that it serves https:// and that --server or OMNI_SERVER is right"
                ))
            });
        }
    }
    Ok(Box::new(rustls::StreamOwned::new(conn, tcp)))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Reads the response in `raw`; `None` when it is not a well-formed one.
fn parse_response(mut raw: Vec<u8>, head_only: bool) -> Option<Response> {
    loop {
        let split = find(&raw, b"\r\n\r\n")?;
        let head = String::from_utf8_lossy(&raw[..split]).into_owned();
        let mut lines = head.lines();
        let status: u16 = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
        let rest = raw.split_off(split + 4);
        // An interim response (such as 100 Continue) is followed by the real one.
        if (100..200).contains(&status) {
            raw = rest;
            continue;
        }
        let headers: Vec<(String, String)> = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(n, v)| (n.trim().to_string(), v.trim().to_string()))
            .collect();
        let header = |name: &str| {
            headers
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
        };
        let mut body = rest;
        if head_only || status == 204 || status == 304 {
            body.clear();
        } else if header("transfer-encoding").is_some_and(|v| v.to_lowercase().contains("chunked"))
        {
            body = decode_chunked(&body)?;
        } else if let Some(length) = header("content-length").and_then(|v| v.parse().ok()) {
            if body.len() < length {
                return None;
            }
            body.truncate(length);
        }
        return Some(Response {
            status,
            headers,
            body,
        });
    }
}

fn decode_chunked(mut raw: &[u8]) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let line_end = find(raw, b"\r\n")?;
        let size_text = std::str::from_utf8(&raw[..line_end]).ok()?;
        let size = usize::from_str_radix(size_text.split(';').next()?.trim(), 16).ok()?;
        raw = &raw[line_end + 2..];
        if size == 0 {
            return Some(body);
        }
        body.extend_from_slice(raw.get(..size)?);
        raw = raw.get(size + 2..)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_server_addresses() {
        let server = Server::parse("http://localhost:8080/").unwrap();
        assert_eq!(server.address(), "localhost:8080");
        let server = Server::parse("http://example.test/api-prefix/").unwrap();
        assert_eq!(server.address(), "example.test:80");
        assert_eq!(server.base, "/api-prefix");
        let server = Server::parse("https://example.test/p").unwrap();
        assert_eq!(server.address(), "example.test:443");
        assert_eq!(
            Server::parse("http://[::1]:9").unwrap().address(),
            "[::1]:9"
        );
        for bad in [
            "localhost:8080",
            "ftp://example.test",
            "http://",
            "http://h:x",
        ] {
            assert!(Server::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn reads_plain_and_chunked_responses() {
        let plain = b"HTTP/1.1 201 Created\r\nContent-Length: 2\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2\r\n\r\nokEXTRA";
        let response = parse_response(plain.to_vec(), false).unwrap();
        assert_eq!((response.status, response.text().as_str()), (201, "ok"));
        assert_eq!(response.headers_named("set-cookie").count(), 2);

        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2;x=1\r\nde\r\n0\r\n\r\n";
        let response = parse_response(chunked.to_vec(), false).unwrap();
        assert_eq!(response.text(), "abcde");

        let interim = b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 204 No Content\r\n\r\n";
        assert_eq!(parse_response(interim.to_vec(), false).unwrap().status, 204);

        assert!(parse_response(b"nonsense".to_vec(), false).is_none());
        assert!(
            parse_response(
                b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\nshort".to_vec(),
                false
            )
            .is_none()
        );
    }
}
