//! Helpers shared by the US-012 API tests: a plain HTTP/1.1 client over a TCP
//! stream (the tests need no extra dependencies) and a server that runs the
//! API's router on a local port inside the test process.
//!
//! The contract these tests pin down:
//!
//! - `GET /api/openapi.json` serves the OpenAPI document. Every operation in it
//!   carries `x-omni-access`, one of `agent`, `human` or `both`, the same value
//!   as the route registry (`omni_api::routes::registry`).
//! - `POST /api/v1/auth/login` takes `{"email", "password"}` and answers
//!   `{"user": {"id", "email"}}` with the session cookie `omni_session`;
//!   `GET /api/v1/auth/me` answers the same body for a valid session cookie;
//!   `POST /api/v1/auth/logout` ends the session and clears the cookie (an
//!   empty value with `Max-Age=0`). A
//!   request without a valid session is 401 `{"error": "unauthorized"}`, bad
//!   credentials are 401 `{"error": "invalid_credentials"}`, and a provider
//!   that cannot answer is 503.
//! - `omni-api setup --email <email> --workspace <name> --password-stdin`
//!   reads the password from the first line of standard input.
#![allow(dead_code)]

use omni_api::auth::AuthProvider;
use omni_api::{AppState, Config};
use serde_json::Value;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

pub const SESSION_COOKIE: &str = "omni_session";

pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Every value of a header that may repeat, such as `Set-Cookie`.
    pub fn headers_named(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
            .collect()
    }

    pub fn content_type(&self) -> &str {
        self.header("content-type").unwrap_or("")
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| {
            panic!(
                "body is not JSON ({e}), status {}: {}",
                self.status,
                self.text()
            )
        })
    }

    /// The whole `Set-Cookie` line of the session cookie, if the response sets it.
    pub fn session_set_cookie(&self) -> Option<&str> {
        let prefix = format!("{SESSION_COOKIE}=");
        self.headers_named("set-cookie")
            .into_iter()
            .find(|line| line.starts_with(&prefix))
    }

    /// The value of the session cookie that the response sets.
    pub fn session_token(&self) -> Option<String> {
        self.session_set_cookie().map(|line| {
            line[SESSION_COOKIE.len() + 1..]
                .split(';')
                .next()
                .unwrap_or("")
                .to_string()
        })
    }
}

/// `Cookie` request header value for a session token.
pub fn cookie(token: &str) -> String {
    format!("{SESSION_COOKIE}={token}")
}

pub struct Client {
    host: String,
    port: u16,
}

impl Client {
    pub fn local(port: u16) -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port,
        }
    }

    /// The reverse proxy of the Compose stack, from OMNI_STACK_URL.
    pub fn proxy() -> Self {
        let url = std::env::var("OMNI_STACK_URL").unwrap_or_else(|_| "http://proxy".to_string());
        let authority = url.trim_start_matches("http://").trim_end_matches('/');
        match authority.split_once(':') {
            Some((host, port)) => Self {
                host: host.to_string(),
                port: port.parse().expect("port in OMNI_STACK_URL"),
            },
            None => Self {
                host: authority.to_string(),
                port: 80,
            },
        }
    }

    pub fn get(&self, path: &str) -> Response {
        self.request("GET", path, &[], None)
    }

    pub fn get_with(&self, path: &str, headers: &[(&str, &str)]) -> Response {
        self.request("GET", path, headers, None)
    }

    pub fn post_json(&self, path: &str, body: &Value, headers: &[(&str, &str)]) -> Response {
        self.request("POST", path, headers, Some(&body.to_string()))
    }

    pub fn request(
        &self,
        method: &str,
        path: &str,
        extra_headers: &[(&str, &str)],
        body: Option<&str>,
    ) -> Response {
        let mut stream = TcpStream::connect((self.host.as_str(), self.port))
            .unwrap_or_else(|e| panic!("cannot connect to {}:{}: {e}", self.host, self.port));
        stream
            .set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        let mut head = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n",
            self.host
        );
        for (name, value) in extra_headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        if let Some(body) = body {
            head.push_str(&format!(
                "Content-Type: application/json\r\nContent-Length: {}\r\n",
                body.len()
            ));
        }
        head.push_str("\r\n");
        stream.write_all(head.as_bytes()).expect("send request");
        if let Some(body) = body {
            stream.write_all(body.as_bytes()).expect("send body");
        }

        let mut raw = Vec::new();
        // A reset after the response is complete is not an error for these tests.
        let _ = stream.read_to_end(&mut raw);
        parse_response(path, &raw)
    }
}

fn parse_response(path: &str, raw: &[u8]) -> Response {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or_else(|| {
            panic!(
                "no HTTP response from {path}: {:?}",
                String::from_utf8_lossy(raw)
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

/// A configuration whose Postgres and RustFS are not there: for tests that
/// must work without touching either.
pub fn offline_config() -> Config {
    Config {
        bind: "127.0.0.1:0".to_string(),
        database_url: "postgres://omni_api:none@127.0.0.1:1/none".to_string(),
        admin_database_url: "postgres://omni:none@127.0.0.1:1/none".to_string(),
        api_db_password: "none".to_string(),
        s3_endpoint: "http://127.0.0.1:1".to_string(),
        s3_access_key: "none".to_string(),
        s3_secret_key: "none".to_string(),
        s3_bucket: "omni".to_string(),
    }
}

/// The API's router served on a local port by a runtime of its own, so the
/// tests themselves stay plain synchronous code.
pub struct TestServer {
    pub client: Client,
    _runtime: tokio::runtime::Runtime,
}

impl TestServer {
    /// Serves the API with its built-in password login.
    pub fn start(config: &Config) -> Self {
        Self::start_with(config, None)
    }

    /// Serves the API with `provider` doing all authentication.
    pub fn start_with_provider(config: &Config, provider: Arc<dyn AuthProvider>) -> Self {
        Self::start_with(config, Some(provider))
    }

    fn start_with(config: &Config, provider: Option<Arc<dyn AuthProvider>>) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime");
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .expect("bind a local port");
        let port = listener.local_addr().unwrap().port();
        let router = {
            // The connection pool needs a runtime to be created in.
            let _guard = runtime.enter();
            let state = match provider {
                Some(provider) => AppState::with_auth_provider(config, provider),
                None => AppState::new(config),
            }
            .expect("build the API state");
            omni_api::router(state)
        };
        runtime.spawn(async move {
            axum::serve(listener, router).await.expect("serve the API");
        });
        Self {
            client: Client::local(port),
            _runtime: runtime,
        }
    }
}
