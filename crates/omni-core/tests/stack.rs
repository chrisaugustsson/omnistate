//! US-010: the running Compose stack, as seen from the test container inside the
//! Compose network. Run with `--features stack-tests`; OMNI_STACK_URL is the
//! reverse proxy's address (default http://proxy). The client is plain
//! HTTP/1.1 over a TCP stream so the tests need no extra dependencies.

use serde_json::Value;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Response {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn content_type(&self) -> &str {
        self.header("content-type").unwrap_or("")
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| {
            panic!(
                "body is not JSON ({e}): {}",
                String::from_utf8_lossy(&self.body)
            )
        })
    }
}

/// Host and port of the reverse proxy, from OMNI_STACK_URL.
fn proxy_address() -> (String, u16) {
    let url = std::env::var("OMNI_STACK_URL").unwrap_or_else(|_| "http://proxy".to_string());
    let authority = url.trim_start_matches("http://").trim_end_matches('/');
    match authority.split_once(':') {
        Some((host, port)) => (
            host.to_string(),
            port.parse().expect("port in OMNI_STACK_URL"),
        ),
        None => (authority.to_string(), 80),
    }
}

fn connect(host: &str, port: u16) -> TcpStream {
    let stream = TcpStream::connect((host, port))
        .unwrap_or_else(|e| panic!("cannot connect to {host}:{port}: {e}"));
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream
}

fn decode_chunked(mut raw: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    loop {
        let line_end = raw
            .windows(2)
            .position(|w| w == b"\r\n")
            .expect("chunk size line");
        let size = usize::from_str_radix(
            std::str::from_utf8(&raw[..line_end])
                .unwrap()
                .split(';')
                .next()
                .unwrap()
                .trim(),
            16,
        )
        .expect("chunk size");
        raw = &raw[line_end + 2..];
        if size == 0 {
            return body;
        }
        body.extend_from_slice(&raw[..size]);
        raw = &raw[size + 2..];
    }
}

/// Sends one request through the proxy and reads the whole response.
fn request(path: &str, extra_headers: &[(&str, &str)]) -> Response {
    let (host, port) = proxy_address();
    let mut stream = connect(&host, port);
    let mut head = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    for (name, value) in extra_headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).expect("send request");

    let mut raw = Vec::new();
    // A reset after the response is complete is not an error for these tests.
    let _ = stream.read_to_end(&mut raw);
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or_else(|| {
            panic!(
                "no HTTP response from {path}: {:?}",
                String::from_utf8_lossy(&raw)
            )
        });
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .expect("status line");
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_string(), v.trim().to_string()))
        .collect();
    let body = raw[split + 4..].to_vec();
    let chunked = headers
        .iter()
        .any(|(n, v)| n.eq_ignore_ascii_case("transfer-encoding") && v.contains("chunked"));
    Response {
        status,
        headers,
        body: if chunked { decode_chunked(&body) } else { body },
    }
}

fn get(path: &str) -> Response {
    request(path, &[])
}

#[test]
fn proxy_serves_the_web_app_at_the_root() {
    let response = get("/");
    assert_eq!(response.status, 200, "GET / through the proxy");
    assert!(
        response.content_type().starts_with("text/html"),
        "GET / is not the web app's HTML page: {}",
        response.content_type()
    );
}

#[test]
fn the_design_renderer_is_revalidated_on_every_load() {
    // The renderer's module and .wasm have fixed URLs; a browser that reused
    // an older build from its cache without asking ran a module that lacked
    // the functions the page calls (US-058's visibleBoards).
    for path in ["/render/omni_render.js", "/render/omni_render.wasm"] {
        let response = get(path);
        assert_eq!(
            response.header("cache-control"),
            Some("no-cache"),
            "GET {path} through the proxy (status {})",
            response.status
        );
    }
}

#[test]
fn web_app_health_route_returns_200_through_the_proxy() {
    let response = get("/health");
    assert_eq!(response.status, 200, "GET /health through the proxy");
}

#[test]
fn api_health_returns_200_json_through_the_proxy() {
    let response = get("/api/health");
    assert_eq!(response.status, 200, "GET /api/health through the proxy");
    assert!(
        response.content_type().starts_with("application/json"),
        "GET /api/health is not answered by the API: {}",
        response.content_type()
    );
    assert_eq!(response.json()["status"], "ok");
}

#[test]
fn api_health_reports_that_postgres_and_the_rustfs_bucket_are_reachable() {
    let health = get("/api/health").json();
    assert_eq!(health["database"], "ok", "API health: {health}");
    assert_eq!(
        health["storage"], "ok",
        "the RustFS bucket must exist on first start: {health}"
    );
}

#[test]
fn proxy_sends_unknown_api_paths_to_the_api_not_the_web_app() {
    let response = get("/api/no-such-route");
    assert_eq!(response.status, 404);
    assert!(
        !response.content_type().starts_with("text/html"),
        "/api/* is answered by the web app: {}",
        response.content_type()
    );
}

#[test]
fn proxy_sends_websocket_upgrades_at_ws_to_the_api() {
    let response = request(
        "/ws/w/00000000-0000-7000-8000-000000000000",
        &[
            ("Upgrade", "websocket"),
            ("Connection", "Upgrade"),
            ("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ=="),
            ("Sec-WebSocket-Version", "13"),
        ],
    );
    // Without credentials the API refuses the workspace's socket. A 404, 502
    // or the web app's HTML page would mean the upgrade never reached the API.
    assert!(
        [101, 401, 403].contains(&response.status),
        "the upgrade did not reach the API: {}",
        response.status
    );
    assert!(
        !response.content_type().starts_with("text/html"),
        "/ws/* is answered by the web app: {}",
        response.content_type()
    );
}

#[test]
fn postgres_and_rustfs_are_reachable_inside_the_compose_network() {
    for (host, port) in [("postgres", 5432), ("rustfs", 9000)] {
        connect(host, port);
    }
}
