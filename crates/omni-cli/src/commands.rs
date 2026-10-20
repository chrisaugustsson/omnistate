//! The commands: `login`, `key list|revoke`, `whoami`, `api` and `config show`;
//! the project and change set commands are in `changesets`, the doc commands in
//! `docs`, the version commands in `versions` and the token commands in
//! `tokens`.
//!
//! Standard output holds only the result of a command (short text, or the API's
//! JSON with `--json`), so a script can parse it. Prompts, notices and errors go
//! to standard error.

use crate::args::Args;
use crate::attach;
use crate::changesets;
use crate::claims;
use crate::config::{self, Stored};
use crate::docs;
use crate::error::{CliError, Result, bail};
use crate::http::{self, Response, Server};
use crate::identity::Identity;
use crate::json::{Value, quote};
use crate::tokens;
use crate::versions;
use crate::wait;
use std::io::{BufRead, IsTerminal, Read, Write};
use std::process::{Command, Stdio};

/// The server that `omni login` uses when nothing else names one: the Compose
/// proxy on its default port.
const DEFAULT_SERVER: &str = "http://localhost:8080";
const SESSION_COOKIE: &str = "omni_session";

pub fn run(args: &Args) -> Result<()> {
    let words: Vec<&str> = args.words.iter().map(String::as_str).collect();
    match words.as_slice() {
        [] => bail!("no command given; see `omni --help`"),
        ["login"] => login(args),
        ["key", "list"] => key(args, KeyAction::List),
        ["key", "revoke", id] => key(args, KeyAction::Revoke((*id).to_string())),
        ["key", "revoke"] => {
            bail!("`omni key revoke` needs the key's ID; `omni key list` shows them")
        }
        ["key", ..] => bail!("usage: omni key list | omni key revoke <key-id>"),
        ["project", "create"] => {
            bail!("`omni project create` needs a name: omni project create <name>")
        }
        ["project", "create", name] => changesets::project_create(args, name),
        ["project", "list"] => changesets::project_list(args),
        ["project", ..] => bail!("usage: omni project create <name> | omni project list"),
        ["changeset", "create"] => changesets::create(args),
        ["changeset", "list"] => changesets::list(args),
        ["changeset", "show", id] => changesets::show(args, id),
        ["changeset", "submit", id] => changesets::act(args, id, "submit"),
        ["changeset", "reopen", id] => changesets::act(args, id, "reopen"),
        ["changeset", "abandon", id] => changesets::act(args, id, "abandon"),
        ["changeset", "rebase", id] => changesets::rebase(args, id),
        ["changeset", "conflicts", id] => changesets::conflicts(args, id),
        ["changeset", "resolve", id] => changesets::resolve(args, id),
        [
            "changeset",
            action @ ("show" | "submit" | "reopen" | "abandon" | "rebase" | "conflicts"),
        ] => {
            bail!(
                "`omni changeset {action}` needs the change set's ID; `omni changeset list` shows them"
            )
        }
        ["changeset", "resolve"] => bail!(
            "`omni changeset resolve` needs the conflict's ID; `omni changeset conflicts <id>` lists them"
        ),
        ["changeset", "reject", ..] => bail!(
            "rejecting a change set is the human's decision and is made in the web app; an agent cannot"
        ),
        ["changeset", ..] => {
            bail!(
                "usage: omni changeset create|list|show|submit|reopen|abandon|rebase|conflicts|resolve; see `omni --help`"
            )
        }
        ["doc", "create"] => docs::create(args),
        ["doc", "open"] => docs::open(args, None),
        ["doc", "open", doc] => docs::open(args, Some(doc)),
        ["doc", "read"] => docs::read(args, None),
        ["doc", "read", doc] => docs::read(args, Some(doc)),
        ["doc", "show"] => docs::show(args, None),
        ["doc", "show", doc] => docs::show(args, Some(doc)),
        ["doc", "edit"] => docs::edit(args, None),
        ["doc", "edit", doc] => docs::edit(args, Some(doc)),
        ["doc", "remove"] => versions::remove(args, None),
        ["doc", "remove", doc] => versions::remove(args, Some(doc)),
        ["doc", ..] => {
            bail!("usage: omni doc create|open|read|show|edit|remove; see `omni --help`")
        }
        ["version", "save"] => versions::save(args, None),
        ["version", "save", doc] => versions::save(args, Some(doc)),
        ["version", "diff"] => versions::diff(args, None, None),
        ["version", "diff", from] => versions::diff(args, Some(from), None),
        ["version", "diff", from, to] => versions::diff(args, Some(from), Some(to)),
        ["version", ..] => bail!("usage: omni version save|diff; see `omni --help`"),
        ["tokens", "import"] => tokens::import(args, None),
        ["tokens", "import", file] => tokens::import(args, Some(file)),
        ["tokens", "export"] => tokens::export(args),
        ["tokens", ..] => bail!(
            "usage: omni tokens import <file> --changeset <id> | omni tokens export [--changeset <id> | --version <v>] [--out <file>] [--per-mode]"
        ),
        ["wait"] => wait::run(args),
        ["wait", ..] => bail!("usage: omni wait --changeset <id> --until <status> [--timeout <s>]"),
        ["claim"] => claims::take(args, None),
        ["claim", node] => claims::take(args, Some(node)),
        ["claim", ..] => bail!("usage: omni claim <node> --changeset <id>"),
        ["release"] => claims::release(args, None),
        ["release", node] => claims::release(args, Some(node)),
        ["release", ..] => bail!("usage: omni release <node> --changeset <id>"),
        ["attach", file] => attach::run(args, file),
        ["attach", ..] => bail!("usage: omni attach <file>"),
        ["whoami"] => whoami(args),
        ["api", method, path] => api(args, method, path),
        ["api", ..] => bail!("usage: omni api <METHOD> <path> [--body-file <path|->]"),
        ["config", "show"] => config_show(args),
        ["config", ..] => bail!("usage: omni config show"),
        ["help"] => {
            say(crate::HELP);
            Ok(())
        }
        [command, ..] => bail!("unknown command {command:?}; see `omni --help`"),
    }
}

/// Prints `text` and a final newline on standard output. A closed pipe is not
/// an error: the reader has what it wanted.
pub fn say(text: &str) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(text.as_bytes());
    if !text.ends_with('\n') {
        let _ = out.write_all(b"\n");
    }
}

// Settings --------------------------------------------------------------------

/// The login with the server overridden by `--server` and `OMNI_SERVER`.
struct Settings {
    stored: Stored,
    server: Option<String>,
}

fn settings(args: &Args) -> Result<Settings> {
    let stored = config::load()?.unwrap_or_default();
    let server = args
        .server
        .clone()
        .or_else(|| std::env::var("OMNI_SERVER").ok())
        .filter(|server| !server.trim().is_empty())
        .or_else(|| stored.server.clone());
    Ok(Settings { stored, server })
}

/// What a command made with an API key needs: where to send it and the headers
/// that authenticate it and say which agent it is.
pub(crate) struct Agent {
    server: Server,
    pub(crate) workspace_id: String,
    headers: Vec<(String, String)>,
}

pub(crate) fn agent(args: &Args) -> Result<Agent> {
    let settings = settings(args)?;
    let (Some(server), Some(workspace_id), Some(api_key)) = (
        settings.server,
        settings.stored.workspace_id,
        settings.stored.api_key,
    ) else {
        bail!("not logged in: run `omni login` to get an API key for a workspace");
    };
    let identity = Identity::resolve(args)?;
    let mut headers = vec![("Authorization".to_string(), format!("Bearer {api_key}"))];
    headers.extend(identity.headers());
    Ok(Agent {
        server: Server::parse(&server)?,
        workspace_id,
        headers,
    })
}

impl Agent {
    pub(crate) fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
    ) -> Result<Response> {
        let json_type = [("Content-Type".to_string(), "application/json".to_string())];
        let extra: &[(String, String)] = if body.is_some() { &json_type } else { &[] };
        self.request_with(method, path, extra, body)
    }

    /// Like `request`, but a 404 is the error `not_found`, for a command that
    /// knows what a missing record means.
    pub(crate) fn request_or_not_found(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        not_found: &str,
    ) -> Result<Response> {
        let mut headers = self.headers.clone();
        if body.is_some() {
            headers.push(("Content-Type".to_string(), "application/json".to_string()));
        }
        let response = http::send(&self.server, method, path, &headers, body)?;
        let status = response.status;
        checked(response).map_err(|error| {
            if status == 404 {
                CliError::new(not_found)
            } else {
                error
            }
        })
    }

    /// Like `request`, but the caller sets the headers of the body itself (the
    /// content type, a file name), for a body that is not JSON.
    pub(crate) fn request_with(
        &self,
        method: &str,
        path: &str,
        extra_headers: &[(String, String)],
        body: Option<&[u8]>,
    ) -> Result<Response> {
        let mut headers = self.headers.clone();
        headers.extend_from_slice(extra_headers);
        checked(http::send(&self.server, method, path, &headers, body)?)
    }
}

/// Shows the response's notices and turns an error status into an error.
fn checked(response: Response) -> Result<Response> {
    for notice in response.headers_named("x-omni-notice") {
        eprintln!("notice: {notice}");
    }
    if response.is_success() {
        Ok(response)
    } else {
        Err(api_error(&response))
    }
}

/// One line for an error response: the API's error code and what to do.
fn api_error(response: &Response) -> CliError {
    let body = Value::parse(&response.text()).ok();
    let code = body
        .as_ref()
        .and_then(|body| body.str_at("error").map(str::to_string))
        .unwrap_or_else(|| format!("http_{}", response.status));
    let status = response.status;
    if let Some(body) = &body {
        if let Some(message) = refused_operation(body) {
            return CliError::new(message);
        }
    }
    CliError::new(match (status, code.as_str()) {
        (401, _) => format!(
            "{code}: the server did not accept the credentials; run `omni login` to get a new key"
        ),
        (403, _) => format!("{code}: only the human can do this, in the web app; an agent cannot"),
        (404, "no_design_system") => format!(
            "{code}: this project has no design system doc; `omni doc create --changeset <id> --kind design-system --module ui --title <text>` starts one"
        ),
        (400, "unknown_mode") => format!(
            "{code}: the design system does not list that mode; `omni tokens export` shows its modes"
        ),
        (404, _) => format!(
            "{code}: there is no such workspace or record, or you cannot reach it; check `omni config show`"
        ),
        (409, "no_mainline_version") => format!(
            "{code}: the doc has no mainline version yet, so only the change set that started it has a draft of it"
        ),
        (503, _) => format!(
            "{code}: the server cannot reach its database or storage right now; try again shortly"
        ),
        _ => format!("{code}: the server answered HTTP {status}"),
    })
}

/// The line for an edit that the API refused: a conflict names the node and
/// the revisions, a refused operation carries the API's own message.
fn refused_operation(body: &Value) -> Option<String> {
    match body.str_at("error")? {
        "conflict" => {
            let conflict = body.get("conflict")?;
            let number = |name: &str| match conflict.get(name) {
                Some(Value::Number(number)) => number.clone(),
                _ => "?".to_string(),
            };
            Some(format!(
                "conflict: operation {} expected node {} at revision {}, but it is at revision {}; read the draft again and send the operation with the new revision",
                number("index"),
                conflict.str_at("node").unwrap_or("?"),
                number("expected"),
                number("current_revision")
            ))
        }
        "claimed" => {
            let message = body.str_at("message").unwrap_or("a claim is in the way");
            Some(match body.get("index") {
                Some(Value::Number(index)) => {
                    format!("claimed: operation {index}: {message}; nothing was applied")
                }
                _ => format!("claimed: {message}"),
            })
        }
        "invalid_tokens" => Some(format!(
            "invalid_tokens: {}; nothing was imported",
            body.str_at("message")
                .unwrap_or("the token tree is refused")
        )),
        "invalid_operation" => {
            let index = match body.get("index") {
                Some(Value::Number(index)) => index.clone(),
                _ => "?".to_string(),
            };
            Some(format!(
                "invalid_operation: operation {index}: {}; nothing was applied",
                body.str_at("message").unwrap_or("the operation is refused")
            ))
        }
        _ => None,
    }
}

// whoami and api ----------------------------------------------------------------

fn whoami(args: &Args) -> Result<()> {
    let agent = agent(args)?;
    let response = agent.request(
        "GET",
        &format!("/api/v1/w/{}/whoami", agent.workspace_id),
        None,
    )?;
    let text = response.text();
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    match Value::parse(&text) {
        Ok(body) => {
            let kind = body
                .get("actor")
                .and_then(|actor| actor.str_at("kind"))
                .unwrap_or("unknown");
            say(&format!(
                "workspace: {}\nactor: {kind}",
                body.str_at("workspace_id").unwrap_or(&agent.workspace_id)
            ));
        }
        Err(_) => say(text.trim_end()),
    }
    Ok(())
}

fn api(args: &Args, method: &str, path: &str) -> Result<()> {
    if method.is_empty() || !method.chars().all(|c| c.is_ascii_alphabetic()) {
        bail!("{method:?} is not an HTTP method; use GET, POST, PUT, PATCH or DELETE");
    }
    let agent = agent(args)?;
    let body = match args.body_file.as_deref() {
        Some(source) => Some(read_body(source)?),
        None => None,
    };
    let response = agent.request(&method.to_ascii_uppercase(), path, body.as_deref())?;
    let text = response.text();
    if !text.is_empty() {
        say(text.trim_end_matches('\n'));
    }
    Ok(())
}

/// The long text of a request: a file, or standard input for `-`.
pub(crate) fn read_body(source: &str) -> Result<Vec<u8>> {
    if source == "-" {
        let mut body = Vec::new();
        std::io::stdin()
            .lock()
            .read_to_end(&mut body)
            .map_err(|e| CliError::new(format!("cannot read the body from standard input: {e}")))?;
        return Ok(body);
    }
    std::fs::read(source)
        .map_err(|e| CliError::new(format!("cannot read the body file {source}: {e}")))
}

// config show -------------------------------------------------------------------

fn config_show(args: &Args) -> Result<()> {
    let settings = settings(args)?;
    let project = config::project()?;
    let path = config::config_path()?;
    let logged_in = settings.stored.api_key.is_some();
    let text_or_null = |value: &Option<String>| value.clone().map_or(Value::Null, Value::String);
    if args.json {
        let fields = [
            ("server", text_or_null(&settings.server)),
            ("workspace_id", text_or_null(&settings.stored.workspace_id)),
            ("email", text_or_null(&settings.stored.email)),
            ("project", text_or_null(&project)),
            ("config_file", Value::String(path.display().to_string())),
            ("logged_in", Value::Bool(logged_in)),
        ];
        let object = Value::Object(
            fields
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        );
        say(&object.to_string());
    } else {
        let show = |value: &Option<String>| value.clone().unwrap_or_else(|| "(none)".to_string());
        say(&format!(
            "server: {}\nworkspace_id: {}\nemail: {}\nproject: {}\nconfig_file: {}\nlogged_in: {}",
            show(&settings.server),
            show(&settings.stored.workspace_id),
            show(&settings.stored.email),
            show(&project),
            path.display(),
            if logged_in { "yes" } else { "no" }
        ));
    }
    Ok(())
}

// Prompts -----------------------------------------------------------------------

fn read_line() -> Result<String> {
    let mut line = String::new();
    let read = std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| CliError::new(format!("cannot read from standard input: {e}")))?;
    if read == 0 {
        bail!("no answer on standard input; run omni in a terminal, or pipe the answers in");
    }
    Ok(line.trim_end_matches(['\n', '\r']).to_string())
}

/// Asks on standard error and reads the answer from standard input. A password
/// is not echoed when standard input is a terminal.
fn ask(prompt: &str, hidden: bool) -> Result<String> {
    let terminal = std::io::stdin().is_terminal();
    eprint!("{prompt}");
    let _ = std::io::stderr().flush();
    let hide = hidden && terminal;
    if hide {
        stty("-echo");
    }
    let answer = read_line();
    if hide {
        stty("echo");
    }
    // The newline that the user's Enter key shows, when it is not shown.
    if hide || !terminal {
        eprintln!();
    }
    answer
}

fn stty(mode: &str) {
    let _ = Command::new("stty")
        .arg(mode)
        .stdin(Stdio::inherit())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

// Acting as the human for one command -----------------------------------------

/// A session of the human, opened for one command and ended with it. It is kept
/// nowhere: not in the config file and not on disk.
struct Session {
    server: Server,
    token: String,
    email: String,
}

impl Session {
    fn open(server: &Server, email: &str, password: &str) -> Result<Session> {
        let body = format!(
            "{{\"email\":{},\"password\":{}}}",
            quote(email),
            quote(password)
        );
        let response = http::send(
            server,
            "POST",
            "/api/v1/auth/login",
            &[("Content-Type".to_string(), "application/json".to_string())],
            Some(body.as_bytes()),
        )?;
        if response.status == 401 {
            bail!("login failed: wrong email or password; check them and try again");
        }
        let response = checked(response)?;
        // The cookie is marked Secure, which a browser would not send over plain
        // http, so the token is read out of it and sent as a Cookie header.
        let token = response
            .headers_named("set-cookie")
            .filter_map(|cookie| cookie.strip_prefix(&format!("{SESSION_COOKIE}=")))
            .filter_map(|rest| rest.split(';').next())
            .find(|token| !token.is_empty())
            .map(str::to_string)
            .ok_or_else(|| {
                CliError::new(
                    "the server did not start a session; is --server pointing at an OmniState API?",
                )
            })?;
        let email = Value::parse(&response.text())
            .ok()
            .and_then(|body| {
                body.get("user")
                    .and_then(|user| user.str_at("email"))
                    .map(str::to_string)
            })
            .unwrap_or_else(|| email.to_string());
        Ok(Session {
            server: server.clone(),
            token,
            email,
        })
    }

    fn request(&self, method: &str, path: &str) -> Result<Response> {
        let headers = [(
            "Cookie".to_string(),
            format!("{SESSION_COOKIE}={}", self.token),
        )];
        checked(http::send(&self.server, method, path, &headers, None)?)
    }

    /// Ends the session. Failing to is not worth an error of its own: the work is done.
    fn close(self) {
        let _ = self.request("POST", "/api/v1/auth/logout");
    }
}

// login -------------------------------------------------------------------------

fn login(args: &Args) -> Result<()> {
    let settings = settings(args)?;
    let server_text = settings
        .server
        .unwrap_or_else(|| DEFAULT_SERVER.to_string());
    let server = Server::parse(&server_text)?;

    let email = ask("Email: ", false)?.trim().to_string();
    if email.is_empty() {
        bail!("the email is empty; run `omni login` again and type it");
    }
    let password = ask("Password: ", true)?;
    let session = Session::open(&server, &email, &password)?;

    let result = login_with(&session, &server_text);
    session.close();
    let done = result?;

    if args.json {
        let fields = [
            ("email", Value::String(done.email)),
            ("workspace_id", Value::String(done.workspace_id)),
            ("workspace_name", Value::String(done.workspace_name)),
            ("server", Value::String(done.server)),
            ("key_prefix", Value::String(done.key_prefix)),
            ("config_file", Value::String(done.path)),
        ];
        let object = Value::Object(
            fields
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        );
        say(&object.to_string());
    } else {
        say(&format!(
            "logged in as {} in workspace {} ({})\nkey {} is stored in {}",
            done.email, done.workspace_name, done.workspace_id, done.key_prefix, done.path
        ));
    }
    Ok(())
}

struct LoggedIn {
    email: String,
    workspace_id: String,
    workspace_name: String,
    server: String,
    key_prefix: String,
    path: String,
}

/// Picks a workspace, creates an API key in it and stores the key.
fn login_with(session: &Session, server_text: &str) -> Result<LoggedIn> {
    let listed = session.request("GET", "/api/v1/auth/workspaces")?;
    let body = Value::parse(&listed.text()).map_err(|_| {
        CliError::new("the server's list of workspaces is not readable; is it an OmniState API?")
    })?;
    let workspaces: Vec<(String, String)> = body
        .get("workspaces")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(|ws| Some((ws.str_at("id")?.to_string(), ws.str_at("name")?.to_string())))
        .collect();

    let (workspace_id, workspace_name) = match workspaces.as_slice() {
        [] => bail!(
            "{} is not a member of any workspace, so there is nothing to log in to",
            session.email
        ),
        [only] => {
            eprintln!("Workspace: {} ({})", only.1, only.0);
            only.clone()
        }
        many => {
            eprintln!("Workspaces:");
            for (number, (id, name)) in many.iter().enumerate() {
                eprintln!("  {}) {name} ({id})", number + 1);
            }
            let answer = ask(&format!("Pick a workspace [1-{}]: ", many.len()), false)?;
            let picked = answer
                .trim()
                .parse::<usize>()
                .ok()
                .filter(|n| (1..=many.len()).contains(n));
            match picked {
                Some(n) => many[n - 1].clone(),
                None => bail!(
                    "{:?} is not a workspace number; run `omni login` again and pick 1 to {}",
                    answer.trim(),
                    many.len()
                ),
            }
        }
    };

    let created = session.request("POST", &format!("/api/v1/w/{workspace_id}/api-keys"))?;
    let created = Value::parse(&created.text())
        .map_err(|_| CliError::new("the server's answer about the new key is not readable"))?;
    let (Some(secret), Some(prefix)) = (
        created.str_at("secret"),
        created.get("key").and_then(|key| key.str_at("prefix")),
    ) else {
        bail!("the server did not return the new key; try `omni login` again");
    };

    let server = server_text.trim().trim_end_matches('/').to_string();
    let path = config::save(&Stored {
        server: Some(server.clone()),
        workspace_id: Some(workspace_id.clone()),
        api_key: Some(secret.to_string()),
        email: Some(session.email.clone()),
    })?;
    Ok(LoggedIn {
        email: session.email.clone(),
        workspace_id,
        workspace_name,
        server,
        key_prefix: prefix.to_string(),
        path: path.display().to_string(),
    })
}

// key list | revoke -------------------------------------------------------------

enum KeyAction {
    List,
    Revoke(String),
}

fn key(args: &Args, action: KeyAction) -> Result<()> {
    if let KeyAction::Revoke(id) = &action
        && (id.len() > 36 || !id.chars().all(|c| c.is_ascii_hexdigit() || c == '-'))
    {
        bail!("{id:?} is not a key ID; `omni key list` shows them");
    }
    let settings = settings(args)?;
    let (Some(server_text), Some(workspace_id)) = (settings.server, settings.stored.workspace_id)
    else {
        bail!("not logged in: run `omni login` first");
    };
    let server = Server::parse(&server_text)?;
    let email = match settings.stored.email {
        Some(email) => email,
        None => ask("Email: ", false)?.trim().to_string(),
    };
    let password = ask(&format!("Password for {email}: "), true)?;

    // The human's session lasts for this command only and is ended with it.
    let session = Session::open(&server, &email, &password)?;
    let result = match action {
        KeyAction::List => session.request("GET", &format!("/api/v1/w/{workspace_id}/api-keys")),
        KeyAction::Revoke(id) => session.request(
            "POST",
            &format!("/api/v1/w/{workspace_id}/api-keys/{id}/revoke"),
        ),
    };
    session.close();
    let response = result?;

    let text = response.text();
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    let body = Value::parse(&text)
        .map_err(|_| CliError::new("the server's answer about the keys is not readable"))?;
    if let Some(keys) = body.get("keys").and_then(Value::as_array) {
        if keys.is_empty() {
            say("no keys");
        }
        for key in keys {
            say(&describe_key(key));
        }
    } else if let Some(key) = body.get("key") {
        say(&format!("revoked {}", describe_key(key)));
    } else {
        say(text.trim_end());
    }
    Ok(())
}

fn describe_key(key: &Value) -> String {
    let field = |name: &str| key.str_at(name).unwrap_or("-");
    let state = match key.str_at("revoked_at") {
        Some(at) => format!("revoked {at}"),
        None => format!(
            "last used {}",
            key.str_at("last_used_at").unwrap_or("never")
        ),
    };
    format!(
        "{}  {}  created {}  {state}",
        field("id"),
        field("prefix"),
        field("created_at")
    )
}
