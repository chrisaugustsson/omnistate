//! Helpers shared by the US-015 CLI tests: a scratch directory, a runner for
//! the built `omni` binary with a clean environment, and a small HTTP server
//! that records the requests it gets.
//!
//! The contract these tests pin down (the rest is the green step's choice):
//!
//! - `omni` reads `$HOME/.config/omni/config.toml`, a TOML file of
//!   `key = "value"` lines: `server` (the base URL, such as
//!   `http://omnistate.localhost`; `--server` and `OMNI_SERVER` override it),
//!   `workspace_id`, `api_key` (the whole secret) and `email`. `omni login`
//!   writes it with file mode 600.
//! - An API key goes in `Authorization: Bearer <secret>`. The agent identity
//!   goes in the headers `X-Omni-Run`, `X-Omni-Role`, `X-Omni-Model` and
//!   `X-Omni-Harness`. `--as <run>/<role>` sets the first two, `--model` and
//!   `--harness` the others; `OMNI_RUN`, `OMNI_ROLE`, `OMNI_MODEL` and
//!   `OMNI_HARNESS` are the fallbacks and the flags win. Run and role are
//!   required; model and harness are optional.
//! - `omni whoami` asks `GET /api/v1/w/{workspace_id}/whoami` as an agent.
//!   `omni api <METHOD> <path> [--body-file <path>]` sends one raw request as
//!   an agent and prints the response body; `--body-file -` reads the body from
//!   standard input. `omni config show` prints the effective settings (never
//!   the key) without any request.
//! - A response header `X-Omni-Notice: <text>` is a notice for the agent: the
//!   CLI prints it on standard error, so a notice reaches the agent without
//!   touching standard output.
//! - Errors: one line on standard error, no output on standard output, and an
//!   exit code that `omni --help` documents under an "Exit codes" heading.
#![allow(dead_code)]

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const WORKSPACE: &str = "01900000-0000-7000-8000-0000000000aa";
pub const API_KEY: &str =
    "omnitest1234_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// A directory that is removed when the value is dropped.
pub struct Scratch {
    pub path: PathBuf,
}

impl Scratch {
    pub fn new(label: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "omni-cli-test-{}-{}-{label}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("create a scratch directory");
        Self { path }
    }

    pub fn join(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

pub fn config_path(home: &Path) -> PathBuf {
    home.join(".config").join("omni").join("config.toml")
}

pub fn links_path(home: &Path) -> PathBuf {
    home.join(".config").join("omni").join("links.toml")
}

/// Writes the config file that `omni login` would, with mode 600.
pub fn write_config(home: &Path, server: &str, workspace_id: &str, api_key: &str) {
    let path = config_path(home);
    std::fs::create_dir_all(path.parent().unwrap()).expect("create the config directory");
    let text = format!(
        "server = \"{server}\"\nworkspace_id = \"{workspace_id}\"\napi_key = \"{api_key}\"\nemail = \"agent@example.test\"\n"
    );
    std::fs::write(&path, text).expect("write the config file");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

/// The value of `key = "value"` in the config file, if there is such a line.
pub fn config_value(home: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(config_path(home)).ok()?;
    text.lines().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        if name.trim() != key {
            return None;
        }
        Some(value.trim().trim_matches('"').to_string())
    })
}

pub fn file_mode(path: &Path) -> u32 {
    std::fs::metadata(path)
        .unwrap_or_else(|e| panic!("cannot stat {}: {e}", path.display()))
        .permissions()
        .mode()
        & 0o777
}

pub struct Out {
    /// The exit code; -1 when the process was killed by a signal.
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
    pub elapsed: Duration,
}

impl Out {
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.stdout).unwrap_or_else(|e| {
            panic!(
                "standard output is not JSON ({e}); stdout: {:?}; stderr: {:?}",
                self.stdout, self.stderr
            )
        })
    }

    /// The non-empty lines of standard error.
    pub fn stderr_lines(&self) -> Vec<&str> {
        non_empty_lines(&self.stderr)
    }
}

pub fn non_empty_lines(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect()
}

/// A run of the built `omni` binary with an empty environment except for HOME
/// and PATH, so nothing of the machine's own settings leaks in.
pub struct Cli {
    args: Vec<String>,
    env: Vec<(String, String)>,
    cwd: PathBuf,
    stdin: Vec<u8>,
}

pub fn omni(home: &Path) -> Cli {
    Cli {
        args: Vec::new(),
        env: vec![("HOME".to_string(), home.display().to_string())],
        cwd: home.to_path_buf(),
        stdin: Vec::new(),
    }
}

impl Cli {
    pub fn args(mut self, args: &[&str]) -> Self {
        self.args.extend(args.iter().map(|a| a.to_string()));
        self
    }

    pub fn env(mut self, name: &str, value: &str) -> Self {
        self.env.push((name.to_string(), value.to_string()));
        self
    }

    pub fn cwd(mut self, cwd: &Path) -> Self {
        self.cwd = cwd.to_path_buf();
        self
    }

    pub fn stdin(mut self, bytes: &[u8]) -> Self {
        self.stdin = bytes.to_vec();
        self
    }

    pub fn run(self) -> Out {
        let mut command = Command::new(env!("CARGO_BIN_EXE_omni"));
        command
            .args(&self.args)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .envs(self.env.iter().map(|(n, v)| (n.as_str(), v.as_str())))
            .current_dir(&self.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let started = Instant::now();
        let mut child = command.spawn().expect("start the omni binary");
        let mut stdin = child.stdin.take().expect("stdin");
        let input = self.stdin;
        // A thread writes the input, so a large body cannot deadlock against
        // the child's output; a child that exits early just closes the pipe.
        let writer = std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
        let output = child.wait_with_output().expect("wait for omni");
        let elapsed = started.elapsed();
        let _ = writer.join();
        Out {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            elapsed,
        }
    }
}

/// The exit codes that `omni --help` documents: the leading numbers of the
/// lines under its "Exit codes" heading.
pub fn documented_exit_codes() -> BTreeSet<i32> {
    let home = Scratch::new("help");
    let help = omni(&home.path).args(&["--help"]).run();
    assert_eq!(help.code, 0, "omni --help: {}", help.stderr);
    let mut codes = BTreeSet::new();
    let mut in_section = false;
    for line in help.stdout.lines() {
        if line.to_lowercase().contains("exit codes") {
            in_section = true;
            continue;
        }
        if !in_section {
            continue;
        }
        let first = line
            .trim()
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .unwrap_or("");
        if let Ok(code) = first.parse() {
            codes.insert(code);
        }
    }
    codes
}

/// A request that the mock server got.
#[derive(Clone, Debug)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Recorded {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Reply {
    pub fn json(status: u16, body: &serde_json::Value) -> Self {
        Self {
            status,
            headers: vec![("Content-Type".to_string(), "application/json".to_string())],
            body: body.to_string(),
        }
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }
}

/// An HTTP server on a local port that answers every request with what the
/// handler returns and records the requests.
pub struct MockServer {
    pub url: String,
    requests: Arc<Mutex<Vec<Recorded>>>,
}

impl MockServer {
    pub fn start(handler: impl Fn(&Recorded) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let handler = Arc::new(handler);
        let log = requests.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let handler = handler.clone();
                let log = log.clone();
                std::thread::spawn(move || serve(stream, &*handler, &log));
            }
        });
        Self {
            url: format!("http://127.0.0.1:{port}"),
            requests,
        }
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }
}

/// A URL on a local port that nothing listens on.
pub fn closed_port_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    format!("http://127.0.0.1:{port}")
}

fn serve(mut stream: TcpStream, handler: &dyn Fn(&Recorded) -> Reply, log: &Mutex<Vec<Recorded>>) {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let Some(request) = read_request(&mut stream) else {
        return;
    };
    let reply = handler(&request);
    log.lock().unwrap().push(request);
    let mut head = format!(
        "HTTP/1.1 {} Reply\r\nContent-Length: {}\r\nConnection: close\r\n",
        reply.status,
        reply.body.len()
    );
    for (name, value) in &reply.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(reply.body.as_bytes());
    let _ = stream.flush();
    let _ = stream.shutdown(std::net::Shutdown::Write);
}

fn read_request(stream: &mut TcpStream) -> Option<Recorded> {
    let mut raw = Vec::new();
    let mut buf = [0u8; 8192];
    let split = loop {
        if let Some(at) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
        let n = stream.read(&mut buf).ok()?;
        if n == 0 {
            return None;
        }
        raw.extend_from_slice(&buf[..n]);
    };
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut lines = head.lines();
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_string();
    let path = first.next()?.to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_string(), v.trim().to_string()))
        .collect();
    let header = |name: &str| {
        headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
    };
    if header("expect").is_some_and(|v| v.eq_ignore_ascii_case("100-continue")) {
        let _ = stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
    }
    let mut body = raw[split + 4..].to_vec();
    if header("transfer-encoding").is_some_and(|v| v.contains("chunked")) {
        while !body.ends_with(b"0\r\n\r\n") {
            let n = stream.read(&mut buf).ok()?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&buf[..n]);
        }
        body = decode_chunked(&body);
    } else {
        let length: usize = header("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        while body.len() < length {
            let n = stream.read(&mut buf).ok()?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&buf[..n]);
        }
    }
    Some(Recorded {
        method,
        path,
        headers,
        body,
    })
}

fn decode_chunked(mut raw: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    loop {
        let Some(line_end) = raw.windows(2).position(|w| w == b"\r\n") else {
            return body;
        };
        let size = std::str::from_utf8(&raw[..line_end])
            .ok()
            .and_then(|s| usize::from_str_radix(s.split(';').next().unwrap_or("").trim(), 16).ok())
            .unwrap_or(0);
        raw = &raw[line_end + 2..];
        if size == 0 || raw.len() < size {
            return body;
        }
        body.extend_from_slice(&raw[..size]);
        raw = raw.get(size + 2..).unwrap_or(&[]);
    }
}
