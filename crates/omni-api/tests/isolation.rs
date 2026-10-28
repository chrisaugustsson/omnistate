//! US-018: the cross-workspace and authority suite. Run with `--features
//! stack-tests` in the Compose test container, which reaches Postgres at
//! `postgres:5432`. It runs in `make test` and on every build from now on, and
//! it attacks every route that the API has, so a story that adds a route (or a
//! tenant table) extends this file in its red step.
//!
//! The suite migrates a scratch database and seeds two workspaces, Alpha (A)
//! and Beta (B), each with a member (alice, bob), a session, an API key, a spare
//! key, two projects and two change sets (one of them rejected by the human),
//! all made through the API itself so that every tenant table has rows of both
//! workspaces. The API's router is served on a local port.
//!
//! The contract these tests pin down:
//!
//! - `CASES` lists every route of `omni_api::routes::registry()`, each either
//!   `Workspace` (its path holds `{workspace_id}`, and the suite attacks it) or
//!   `Global` (no workspace in the path, with the reason). The path may hold
//!   `{project_id}`, `{changeset_id}`, `{key_id}`, `{attachment_id}`,
//!   `{doc_id}`, `{version_id}`, `{other_version_id}`, `{conflict_id}`,
//!   `{thread_id}`, `{post_id}`, `{decision_id}`, `{comment_id}` and
//!   `{feature_id}`, which the suite fills with the IDs of the seeded
//!   records, and `{board_id}`, which it fills with a node ID that no board
//!   has (US-030); a path with another placeholder, or a registry route
//!   without a case, fails the suite. A body may hold `<claimed_node>`,
//!   which the suite fills with the section that the record's workspace
//!   claimed (see below), so that an attack on the claim routes names a node
//!   that really is claimed, `<comment_node>`, the node of `{version_id}`
//!   that the seeded comment is pinned to, and `<other_version_id>`, the
//!   record's `{other_version_id}`.
//!   The seeded doc (US-022) was created in the open change set `{changeset_id}`,
//!   which has its draft, and has one applied operation; once the claim routes
//!   exist (US-023), the draft also has a section that the workspace's agent
//!   claimed, and once the version routes exist (US-024), the draft was saved
//!   as `{version_id}`, edited and saved again as `{other_version_id}`. Once
//!   the approve route exists (US-025), each workspace also has a change set
//!   with a saved version of a doc of its own that the human approved and that
//!   merged, so whatever records approvals has rows of both workspaces. Once
//!   the rebase route exists (US-026), `{changeset_id}` was also submitted,
//!   approved and rebased with a conflict on a doc whose mainline moved on
//!   under it, which left it `open` again with the conflict `{conflict_id}`.
//!   Once the thread routes exist (US-033), `{changeset_id}` also has the
//!   thread `{thread_id}`, which the workspace's agent started, moved on to a
//!   sealed second phase and posted in; that post, which the workspace's
//!   agent (the API key without identity headers) wrote, is `{post_id}`, and
//!   its summary is text that nothing else holds. Once the directive route exists
//!   (US-034), the human also posted a directive in it, which the workspace's
//!   agent was given with its next request in the change set. Once the
//!   decision routes exist (US-037), the workspace's agent also proposed the
//!   decision `{decision_id}` for that thread, relying on `{post_id}`, whose
//!   direction is text that nothing else holds, and linked `{changeset_id}` to
//!   it; the decision is still `proposed`. Once the comment routes exist
//!   (US-038), the human also commented on the paragraph of `{version_id}`
//!   (the comment `{comment_id}`, whose text nothing else holds) and the
//!   workspace's agent replied to it (and, while comments had a status,
//!   marked it addressed in `{other_version_id}`; US-106 removed statuses). Once the feature routes exist (US-059), the
//!   workspace's agent also created the feature `{feature_id}` in
//!   `{project_id}`, whose name is text that nothing else holds. The doc list
//!   (US-061) lists `{doc_id}` among the project's docs. Once the scope-level
//!   doc routes exist (US-102), the workspace's agent also created a doc at the
//!   app level of `{project_id}` and one in `{feature_id}` without a change
//!   set, so the log of the writes to docs (`doc_changes`) has rows of the
//!   workspace. The rename and delete routes of docs and features (US-110)
//!   are attacked with the seeded `{doc_id}` and `{feature_id}`; their
//!   controls rename and delete records of their own.
//! - Workspace A's credentials against workspace B's workspace and IDs, and
//!   against B's IDs under A's own workspace, are always 404
//!   `{"error": "not_found"}` with nothing of B in the body, and the answer is
//!   the one for an ID that does not exist. The caller's authority is checked
//!   first (FR-5): an API key on a `human` route is 403 whatever the path says,
//!   so the attack on a `human` route uses alice's session.
//! - Every `human` route (the login, which takes a password and not a key, is
//!   the one exemption) answers an API key with 403 `{"error": "forbidden"}`
//!   and changes nothing.
//! - `omni_api::routes::websocket_registry()` lists the WebSocket routes, as
//!   `registry()` lists the HTTP ones; the suite covers each of them, and every
//!   path that the router source serves is in one of the two registries.
//! - At the database, the API role (`omni_api`) sees none of the other
//!   workspace's rows in any tenant table, and cannot insert, update, move or
//!   delete them.

mod common;

use common::{Client, Response, TestServer, cookie};
use omni_api::Config;
use omni_api::routes::{Access, RouteEntry, registry, websocket_registry};
use s3::creds::Credentials;
use s3::{Bucket, BucketConfiguration, Region};
use serde_json::{Value, json};
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::collections::BTreeSet;
use std::future::Future;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

const TEST_DB: &str = "omni_us018_isolation";
/// The scratch bucket of the attachments that the suite seeds (US-019).
const TEST_BUCKET: &str = "omni-us018-isolation";
/// An ID that no row has.
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-000000000000";
/// A node ID in the right format that no node has.
const UNKNOWN_NODE: &str = "zzzzzzzz";
/// The handshake key of RFC 6455's example.
const HANDSHAKE_KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";
/// How long a test waits for something that must happen.
const PATIENCE: Duration = Duration::from_secs(5);
/// How long a test waits to be sure that something does not happen.
const QUIET: Duration = Duration::from_millis(600);

// ---------------------------------------------------------------------------
// The coverage table
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Kind {
    /// The path holds `{workspace_id}`: the suite attacks it.
    Workspace,
    /// No workspace in the path, so there is no other workspace to reach.
    Global(&'static str),
}

struct Case {
    method: &'static str,
    path: &'static str,
    /// The JSON body of a call that would succeed if the caller were allowed.
    body: Option<&'static str>,
    kind: Kind,
}

const fn case(
    method: &'static str,
    path: &'static str,
    body: Option<&'static str>,
    kind: Kind,
) -> Case {
    Case {
        method,
        path,
        body,
        kind,
    }
}

const CASES: &[Case] = &[
    case(
        "GET",
        "/api/health",
        None,
        Kind::Global("reports the stack; no workspace"),
    ),
    case(
        "GET",
        "/api/openapi.json",
        None,
        Kind::Global("the public API description; no workspace"),
    ),
    case(
        "POST",
        "/api/v1/auth/login",
        Some(r#"{"email":"alice@example.test","password":"wrong"}"#),
        Kind::Global("takes an email and a password; no workspace"),
    ),
    case(
        "POST",
        "/api/v1/auth/logout",
        None,
        Kind::Global("ends the caller's own session; no workspace"),
    ),
    case(
        "GET",
        "/api/v1/auth/me",
        None,
        Kind::Global("the caller's own user; no workspace"),
    ),
    case(
        "GET",
        "/api/v1/auth/workspaces",
        None,
        Kind::Global("the caller's own memberships; no workspace"),
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/whoami",
        None,
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/api-keys",
        None,
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/api-keys",
        Some("{}"),
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/api-keys/{key_id}/revoke",
        None,
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects",
        None,
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects",
        Some(r#"{"name":"intruder project"}"#),
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets",
        None,
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets",
        Some(r#"{"title":"intruder change","run_tag":"run-1"}"#),
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}",
        None,
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/submit",
        None,
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/reject",
        Some(r#"{"reason":"not good enough yet"}"#),
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/reopen",
        None,
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/abandon",
        None,
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/attachments",
        Some("the bytes of a file"),
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/attachments/{attachment_id}",
        None,
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs",
        Some(r#"{"kind":"html","pillar":"spec","title":"intruder doc"}"#),
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}",
        None,
        Kind::Workspace,
    ),
    case(
        "PATCH",
        "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}",
        Some(r#"{"title":"intruder"}"#),
        Kind::Workspace,
    ),
    case(
        "DELETE",
        "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}",
        None,
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/html",
        None,
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/versions",
        None,
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/open",
        None,
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}",
        None,
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/ops",
        Some(
            r#"{"ops":[{"op":"insert","parent":null,"position":{"after":null},"content":"<p>intruder</p>"}]}"#,
        ),
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/claims",
        Some(r#"{"node":"<claimed_node>"}"#),
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/claims/release",
        Some(r#"{"node":"<claimed_node>"}"#),
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/versions",
        Some("{}"),
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/remove",
        Some("{}"),
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/versions/{version_id}",
        None,
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/versions/{version_id}/diff/{other_version_id}",
        None,
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/approve",
        Some("{}"),
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/rebase",
        Some("{}"),
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/conflicts",
        None,
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/conflicts/{conflict_id}/resolve",
        Some(r#"{"note":"intruder"}"#),
        Kind::Workspace,
    ),
    case(
        "PUT",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/tokens",
        Some(
            r##"{"tokens":{"$extensions":{"omnistate":{"modes":["light"]}},"color":{"$type":"color","intruder":{"$value":"#000000"}}}}"##,
        ),
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/tokens",
        None,
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/tokens",
        None,
        Kind::Workspace,
    ),
    case(
        "PUT",
        "/api/v1/w/{workspace_id}/projects/{project_id}/tokens",
        Some(
            r##"{"tokens":{"$extensions":{"omnistate":{"modes":["light"]}},"color":{"$type":"color","intruder":{"$value":"#000000"}}}}"##,
        ),
        Kind::Workspace,
    ),
    case(
        "PUT",
        "/api/v1/w/{workspace_id}/projects/{project_id}/components",
        Some(
            r##"{"tags":"<Component name=\"Intruder\" dir=\"row\" fill=\"#000000\" props={{label: {type: \"text\", default: \"intruder\"}}}>\n  <Text name=\"Intruder label\" content=\"intruder\" fontFamily=\"Inter\" fontSize={12} bind={{content: \"label\"}}/>\n</Component>"}"##,
        ),
        Kind::Workspace,
    ),
    case(
        "PUT",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/components",
        Some(
            r##"{"tags":"<Component name=\"Intruder\" dir=\"row\" fill=\"#000000\" props={{label: {type: \"text\", default: \"intruder\"}}}>\n  <Text name=\"Intruder label\" content=\"intruder\" fontFamily=\"Inter\" fontSize={12} bind={{content: \"label\"}}/>\n</Component>"}"##,
        ),
        Kind::Workspace,
    ),
    case(
        "PUT",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/design",
        Some(
            r#"{"tags":"<Page name=\"Intruder\">\n  <Board name=\"Intruder board\" x={0} y={0} width={320} height={200}/>\n</Page>"}"#,
        ),
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/components/usage",
        None,
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/boards/{board_id}",
        None,
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/boards/{board_id}",
        None,
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/threads",
        Some(r#"{"question":"intruder","criteria":["intruder"]}"#),
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}",
        None,
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/phases",
        Some(r#"{"sealed":true}"#),
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/posts",
        Some(r#"{"type":"question","summary":"intruder","body":"intruder"}"#),
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/threads",
        Some(r#"{"question":"intruder"}"#),
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/threads",
        Some(r#"{"question":"intruder","sealed":true}"#),
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/threads",
        None,
        Kind::Workspace,
    ),
    case(
        "PUT",
        "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/conclusion",
        Some(r#"{"conclusion":"intruder"}"#),
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/directives",
        Some(r#"{"summary":"intruder","body":"intruder"}"#),
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/digest",
        None,
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/posts/{post_id}",
        None,
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/decisions",
        Some(
            r#"{"direction":"intruder","chosen":"intruder","criteria":["intruder"],"options":[{"name":"intruder","scores":{"intruder":1},"lost_because":null}],"assumptions":[],"open_questions":[],"dissent":[],"posts":[]}"#,
        ),
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/decisions/{decision_id}",
        None,
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/decisions/{decision_id}/approve",
        Some("{}"),
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/decisions/{decision_id}/reject",
        Some(r#"{"reason":"intruder"}"#),
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/decisions/{decision_id}",
        None,
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/versions/{version_id}/comments",
        Some(r#"{"node":"<comment_node>","text":"intruder"}"#),
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/comments",
        None,
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/comments/{comment_id}",
        None,
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/comments/{comment_id}/replies",
        Some(r#"{"text":"intruder"}"#),
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/comments",
        Some(r#"{"node":"<comment_node>","text":"intruder"}"#),
        Kind::Workspace,
    ),
    case(
        "DELETE",
        "/api/v1/w/{workspace_id}/projects/{project_id}/comments/{comment_id}",
        None,
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/home",
        None,
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/review",
        None,
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/features",
        Some(r#"{"name":"intruder","description":"intruder"}"#),
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/features",
        None,
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}",
        None,
        Kind::Workspace,
    ),
    case(
        "PATCH",
        "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}",
        Some(r#"{"name":"intruder"}"#),
        Kind::Workspace,
    ),
    case(
        "DELETE",
        "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}",
        None,
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/docs",
        None,
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/docs",
        Some(r#"{"kind":"html","pillar":"spec","title":"intruder"}"#),
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/docs",
        Some(r#"{"kind":"html","pillar":"spec","title":"intruder"}"#),
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/ops",
        Some(
            r#"{"ops":[{"op":"insert","parent":null,"position":{"after":null},"content":"<p>intruder</p>"}]}"#,
        ),
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/activity",
        None,
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/activity/{seq}",
        None,
        Kind::Workspace,
    ),
];

/// The WebSocket routes that the suite attacks, as the paths of
/// `websocket_registry()`.
const WEBSOCKET_PATHS: &[&str] = &["/ws/w/{workspace_id}"];

/// The human routes that an API key may still reach: the login checks a
/// password, so a key next to it is not an attempt at the human's authority.
const KEY_EXEMPT: &[(&str, &str)] = &[("POST", "/api/v1/auth/login")];

fn route_name(method: &str, path: &str) -> String {
    format!("{method} {path}")
}

/// The routes of `routes` that no case covers.
fn uncovered(routes: &[RouteEntry], cases: &[Case]) -> Vec<String> {
    routes
        .iter()
        .filter(|r| {
            !cases
                .iter()
                .any(|c| c.method == r.method && c.path == r.path)
        })
        .map(|r| route_name(&r.method, &r.path))
        .collect()
}

/// The cases that match no route of `routes`.
fn stale(routes: &[RouteEntry], cases: &[Case]) -> Vec<String> {
    cases
        .iter()
        .filter(|c| {
            !routes
                .iter()
                .any(|r| c.method == r.method && c.path == r.path)
        })
        .map(|c| route_name(c.method, c.path))
        .collect()
}

fn case_for(entry: &RouteEntry) -> &'static Case {
    CASES
        .iter()
        .find(|c| c.method == entry.method && c.path == entry.path)
        .unwrap_or_else(|| {
            panic!(
                "no suite coverage for {}: add a case to CASES in tests/isolation.rs",
                route_name(&entry.method, &entry.path)
            )
        })
}

fn case_named(method: &str, path: &str) -> &'static Case {
    CASES
        .iter()
        .find(|c| c.method == method && c.path == path)
        .unwrap_or_else(|| panic!("no case for {}", route_name(method, path)))
}

// ---------------------------------------------------------------------------
// Plumbing
// ---------------------------------------------------------------------------

fn block<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
        .block_on(future)
}

fn admin_url() -> String {
    std::env::var("OMNI_ADMIN_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://omni:omni-dev@postgres:5432/omni".to_string())
}

fn admin_options() -> PgConnectOptions {
    PgConnectOptions::from_str(&admin_url()).expect("OMNI_ADMIN_DATABASE_URL")
}

fn api_password() -> String {
    // The password that the stack's API gives omni_api (docker-compose.yml). The
    // role is cluster-wide, so the scratch database must use the same one.
    std::env::var("OMNI_API_DB_PASSWORD").unwrap_or_else(|_| "omni-api-dev".to_string())
}

fn config() -> Config {
    let admin = admin_options();
    let url = admin_url();
    let (server, _database) = url.rsplit_once('/').expect("OMNI_ADMIN_DATABASE_URL");
    Config {
        bind: "127.0.0.1:0".to_string(),
        database_url: format!(
            "postgres://omni_api:{}@{}:{}/{TEST_DB}",
            api_password(),
            admin.get_host(),
            admin.get_port()
        ),
        admin_database_url: format!("{server}/{TEST_DB}"),
        api_db_password: api_password(),
        // The attachments (US-019) keep their bytes in RustFS.
        s3_endpoint: s3_endpoint(),
        s3_access_key: s3_access_key(),
        s3_secret_key: s3_secret_key(),
        s3_bucket: TEST_BUCKET.to_string(),
    }
}

fn s3_endpoint() -> String {
    std::env::var("OMNI_S3_ENDPOINT").unwrap_or_else(|_| "http://rustfs:9000".to_string())
}

fn s3_access_key() -> String {
    std::env::var("OMNI_S3_ACCESS_KEY").unwrap_or_else(|_| "omni-dev".to_string())
}

fn s3_secret_key() -> String {
    std::env::var("OMNI_S3_SECRET_KEY").unwrap_or_else(|_| "omni-dev-secret".to_string())
}

fn s3_region() -> Region {
    Region::Custom {
        region: "us-east-1".to_string(),
        endpoint: s3_endpoint(),
    }
}

fn s3_credentials() -> Credentials {
    Credentials::new(
        Some(&s3_access_key()),
        Some(&s3_secret_key()),
        None,
        None,
        None,
    )
    .expect("storage credentials")
}

fn bucket() -> Box<Bucket> {
    Bucket::new(TEST_BUCKET, s3_region(), s3_credentials())
        .expect("bucket settings")
        .with_path_style()
}

/// The keys in the scratch bucket under `prefix`, sorted.
fn keys_under(prefix: &str) -> Vec<String> {
    let pages = block(bucket().list(prefix.to_string(), None))
        .unwrap_or_else(|e| panic!("cannot list {prefix}: {e}"));
    let mut keys: Vec<String> = pages
        .into_iter()
        .flat_map(|page| page.contents)
        .map(|object| object.key)
        .collect();
    keys.sort();
    keys
}

async fn admin_connection() -> PgConnection {
    PgConnection::connect_with(&admin_options().database(TEST_DB))
        .await
        .expect("connect to the scratch database as the admin role")
}

/// A connection that logs in as the API role itself, as the API's pool does.
async fn api_connection() -> PgConnection {
    PgConnection::connect(&config().database_url)
        .await
        .expect("connect to the scratch database as the API role")
}

fn unique(label: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!("{label}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// Tests that change workspace B on purpose, and tests that check B did not
/// change, take this lock so they do not see each other's work.
fn serial() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// How a request authenticates: an API key (an agent), a session cookie (the
/// human), both at once, or nothing.
#[derive(Clone, Copy)]
enum Auth<'a> {
    Key(&'a str),
    Session(&'a str),
    KeyAndSession(&'a str, &'a str),
    Nobody,
}

impl Auth<'_> {
    fn headers(&self) -> Vec<(&'static str, String)> {
        let key = |secret: &str| ("Authorization", format!("Bearer {secret}"));
        // An agent declares its run and role, which the post routes need
        // (US-055).
        let identity = || {
            [
                ("X-Omni-Run", "run-1".to_string()),
                ("X-Omni-Role", "agent".to_string()),
            ]
        };
        match self {
            Auth::Key(secret) => [vec![key(secret)], identity().to_vec()].concat(),
            Auth::Session(token) => vec![("Cookie", cookie(token))],
            Auth::KeyAndSession(secret, token) => [
                vec![key(secret), ("Cookie", cookie(token))],
                identity().to_vec(),
            ]
            .concat(),
            Auth::Nobody => vec![],
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Auth::Key(_) => "an API key",
            Auth::Session(_) => "a session",
            Auth::KeyAndSession(..) => "an API key and a session",
            Auth::Nobody => "no credentials",
        }
    }
}

fn call_with(
    client: &Client,
    method: &str,
    path: &str,
    auth: &Auth,
    extra: &[(&str, &str)],
    body: Option<&str>,
) -> Response {
    let owned = auth.headers();
    let mut headers: Vec<(&str, &str)> = owned.iter().map(|(n, v)| (*n, v.as_str())).collect();
    headers.extend_from_slice(extra);
    client.request(method, path, &headers, body)
}

fn call(client: &Client, method: &str, path: &str, auth: &Auth, body: Option<&str>) -> Response {
    call_with(client, method, path, auth, &[], body)
}

/// A response that must have `status`; returns its JSON body.
fn must(response: Response, status: u16, what: &str) -> Value {
    assert_eq!(response.status, status, "{what}: {}", response.text());
    response.json()
}

/// Whether the router has the attachment routes (US-019) yet.
fn has_attachment_routes() -> bool {
    registry()
        .iter()
        .any(|r| r.method == "POST" && r.path == "/api/v1/w/{workspace_id}/attachments")
}

/// Whether the router has the doc and draft routes (US-022) yet.
fn has_doc_routes() -> bool {
    registry().iter().any(|r| {
        r.method == "POST"
            && r.path
                == "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs"
    })
}

/// Whether the router has the claim routes (US-023) yet.
fn has_claim_routes() -> bool {
    registry().iter().any(|r| {
        r.method == "POST"
            && r.path
                == "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/claims"
    })
}

/// Whether the router has the version routes (US-024) yet.
fn has_version_routes() -> bool {
    registry().iter().any(|r| {
        r.method == "POST"
            && r.path
                == "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/versions"
    })
}

/// Whether the router has the approve route (US-025) yet.
fn has_approve_route() -> bool {
    registry().iter().any(|r| {
        r.method == "POST"
            && r.path
                == "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/approve"
    })
}

/// Whether the router has the rebase route (US-026) yet.
fn has_rebase_route() -> bool {
    registry().iter().any(|r| {
        r.method == "POST"
            && r.path
                == "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/rebase"
    })
}

/// Whether the router has the thread routes (US-033) yet.
fn has_thread_routes() -> bool {
    registry().iter().any(|r| {
        r.method == "POST"
            && r.path
                == "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/threads"
    })
}

/// Whether the router has the directive route (US-034) yet.
fn has_directive_route() -> bool {
    registry().iter().any(|r| {
        r.method == "POST"
            && r.path
                == "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/directives"
    })
}

/// Whether the router has the decision routes (US-037) yet.
fn has_decision_routes() -> bool {
    registry().iter().any(|r| {
        r.method == "POST"
            && r.path
                == "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/decisions"
    })
}

/// Whether the router has the comment routes (US-038) yet.
fn has_comment_routes() -> bool {
    registry().iter().any(|r| {
        r.method == "POST"
            && r.path
                == "/api/v1/w/{workspace_id}/projects/{project_id}/versions/{version_id}/comments"
    })
}

/// Whether the registry has the route `method path`.
fn has_route(method: &str, path: &str) -> bool {
    registry()
        .iter()
        .any(|r| r.method == method && r.path == path)
}

/// Whether the router has the feature routes (US-059) yet.
fn has_feature_routes() -> bool {
    registry().iter().any(|r| {
        r.method == "POST" && r.path == "/api/v1/w/{workspace_id}/projects/{project_id}/features"
    })
}

/// Whether the router has the scope-level doc routes (US-102) yet.
fn has_live_doc_routes() -> bool {
    registry().iter().any(|r| {
        r.method == "POST"
            && r.path == "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/docs"
    })
}

/// The `data-rev` of the block with `data-omni-id="{node}"` in a draft's HTML.
fn rev_of(html: &str, node: &str) -> u64 {
    let at = html
        .find(&format!("data-omni-id=\"{node}\""))
        .unwrap_or_else(|| panic!("the draft has {node}: {html}"));
    let start = html[..at].rfind('<').expect("the start of the tag");
    let end = at + html[at..].find('>').expect("the end of the tag");
    let tag = &html[start..end];
    let rev = tag
        .find("data-rev=\"")
        .map(|i| &tag[i + "data-rev=\"".len()..])
        .and_then(|rest| rest.split('"').next())
        .unwrap_or_else(|| panic!("{node} has a data-rev: {tag}"));
    rev.parse().expect("a revision")
}

/// Leaves a conflict in `target`, an open change set of the project whose
/// change sets are at `changesets` (US-026): a doc with one paragraph merges,
/// another change set rewrites the paragraph and merges, `target` rewrites it
/// too from the same base, the human approves it, it needs a rebase, and the
/// agent rebases it with the body of the rebase case. Returns the conflict's ID.
fn seed_conflict(
    client: &Client,
    changesets: &str,
    agent: &Auth,
    human: &Auth,
    target: &str,
    label: &str,
) -> String {
    let changeset = || -> String {
        let body = json!({ "title": unique(&format!("{label}-change")) }).to_string();
        let created = must(
            call(client, "POST", changesets, agent, Some(&body)),
            201,
            "create a change set",
        );
        created["changeset"]["id"]
            .as_str()
            .expect("an id")
            .to_string()
    };
    let submit_and_approve = |id: &str| -> Value {
        let path = format!("{changesets}/{id}");
        must(
            call(client, "POST", &format!("{path}/submit"), agent, None),
            200,
            "submit",
        );
        must(
            call(
                client,
                "POST",
                &format!("{path}/approve"),
                human,
                Some("{}"),
            ),
            200,
            "approve",
        )
    };
    let save = |draft: &str| {
        must(
            call(
                client,
                "POST",
                &format!("{draft}/versions"),
                agent,
                Some("{}"),
            ),
            201,
            "save a version",
        );
    };

    // A doc with one paragraph on the mainline.
    let first = changeset();
    let body = json!({ "kind": "html", "pillar": "spec", "title": unique(&format!("{label}-rebased-doc")) })
        .to_string();
    let created = must(
        call(
            client,
            "POST",
            &format!("{changesets}/{first}/docs"),
            agent,
            Some(&body),
        ),
        201,
        "create a doc to rebase",
    );
    let doc = created["doc"]["id"].as_str().expect("an id").to_string();
    let ops = json!({ "ops": [{
        "op": "insert",
        "parent": null,
        "position": { "after": null },
        "content": format!("<p>{}</p>", unique(&format!("{label}-base")))
    }] })
    .to_string();
    let inserted = must(
        call(
            client,
            "POST",
            &format!("{changesets}/{first}/docs/{doc}/ops"),
            agent,
            Some(&ops),
        ),
        200,
        "write the paragraph",
    );
    let paragraph = inserted["results"][0]["node"]
        .as_str()
        .expect("the new paragraph")
        .to_string();
    save(&format!("{changesets}/{first}/docs/{doc}"));
    let merged = submit_and_approve(&first);
    assert_eq!(merged["changeset"]["status"], "merged", "{merged}");

    // Two change sets rewrite it from the same base; one merges first.
    let ahead = changeset();
    for id in [ahead.as_str(), target] {
        let opened = call(
            client,
            "POST",
            &format!("{changesets}/{id}/docs/{doc}/open"),
            agent,
            Some("{}"),
        );
        assert!(
            matches!(opened.status, 200 | 201),
            "open the doc: {}",
            opened.text()
        );
    }
    for id in [ahead.as_str(), target] {
        let draft = format!("{changesets}/{id}/docs/{doc}");
        let read = must(call(client, "GET", &draft, agent, None), 200, "read");
        let html = read["draft"]["html"].as_str().expect("the draft's html");
        let ops = json!({ "ops": [{
            "op": "set",
            "node": paragraph,
            "rev": rev_of(html, &paragraph),
            "properties": { "html": unique(&format!("{label}-rewrite")) }
        }] })
        .to_string();
        must(
            call(client, "POST", &format!("{draft}/ops"), agent, Some(&ops)),
            200,
            "rewrite the paragraph",
        );
        save(&draft);
    }
    let merged = submit_and_approve(&ahead);
    assert_eq!(merged["changeset"]["status"], "merged", "{merged}");
    let behind = submit_and_approve(target);
    assert_eq!(behind["changeset"]["status"], "needs_rebase", "{behind}");

    let rebase = case_named(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/rebase",
    );
    let rebased = must(
        call(
            client,
            "POST",
            &format!("{changesets}/{target}/rebase"),
            agent,
            rebase.body,
        ),
        200,
        "rebase",
    );
    assert_eq!(rebased["changeset"]["status"], "open", "{rebased}");
    rebased["conflicts"][0]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("the rebase left a conflict: {rebased}"))
        .to_string()
}

/// Uploads `bytes` as `name` with an API key; the raw bytes are the body.
fn upload_attachment(
    client: &Client,
    workspace: &str,
    key: &str,
    bytes: &[u8],
    name: &str,
) -> Response {
    let bearer = format!("Bearer {key}");
    client.request_bytes(
        "POST",
        &format!("/api/v1/w/{workspace}/attachments"),
        &[
            ("Authorization", bearer.as_str()),
            ("Content-Type", "text/plain"),
            ("X-Omni-Filename", name),
        ],
        Some(bytes),
    )
}

/// One workspace as the suite sees it: who belongs to it and which records it
/// has. The fields of a stranger hold one made-up ID.
struct Side {
    id: String,
    email: String,
    token: String,
    /// The key that the workspace's tests use; never revoked.
    key: String,
    /// A second key, which is the target of the revoke attacks.
    key_id: String,
    key_prefix: String,
    project_id: String,
    project_name: String,
    other_project_id: String,
    changeset_id: String,
    rejected_changeset_id: String,
    /// A file that the workspace's agent uploaded (US-019): empty until the
    /// attachment routes exist.
    attachment_id: String,
    attachment_name: String,
    /// The file's content, which is text that nothing else holds.
    attachment_text: String,
    /// A doc that the workspace's agent started in `changeset_id` (US-022),
    /// with one applied operation: empty until the doc routes exist.
    doc_id: String,
    doc_title: String,
    /// The text of the doc's one paragraph, which nothing else holds.
    doc_text: String,
    /// A section of the doc's draft that the workspace's agent claimed and
    /// holds (US-023): empty until the claim routes exist.
    claimed_node: String,
    /// Two saved versions of the doc's draft (US-024), the second after one
    /// more paragraph: empty until the version routes exist.
    version_id: String,
    other_version_id: String,
    /// A change set with a saved version of a doc of its own, which the human
    /// approved and which merged (US-025): empty until the approve route
    /// exists.
    merged_changeset_id: String,
    /// The conflict that a rebase left in `changeset_id` (US-026): empty
    /// until the rebase route exists.
    conflict_id: String,
    /// A thread of `changeset_id` that the workspace's agent started, moved
    /// on to a sealed second phase and posted in (US-033), and its question,
    /// which nothing else holds: empty until the thread routes exist.
    thread_id: String,
    thread_question: String,
    /// The post that the workspace's agent made in the thread's sealed second
    /// phase (US-035), and its summary, which nothing else holds: empty
    /// until the thread routes exist.
    post_id: String,
    post_summary: String,
    /// The decision that the workspace's agent proposed for the thread and
    /// linked `changeset_id` to (US-037), and its direction, which nothing
    /// else holds: empty until the decision routes exist.
    decision_id: String,
    decision_direction: String,
    /// The comment that the human made on the paragraph of `version_id`, to
    /// which the workspace's agent replied and which it marked addressed in
    /// `other_version_id` (US-038), the paragraph's node and the comment's
    /// text, which nothing else holds: empty until the comment routes exist.
    comment_id: String,
    comment_node: String,
    comment_text: String,
    /// The feature that the workspace's agent created in `project_id`
    /// (US-059), and its name, which nothing else holds: empty until the
    /// feature routes exist.
    feature_id: String,
    feature_name: String,
}

impl Side {
    /// A "workspace" whose every ID is `id`: one that does not exist.
    fn stranger(id: &str) -> Side {
        let same = || id.to_string();
        Side {
            id: same(),
            email: String::new(),
            token: String::new(),
            key: String::new(),
            key_id: same(),
            key_prefix: String::new(),
            project_id: same(),
            project_name: String::new(),
            other_project_id: same(),
            changeset_id: same(),
            rejected_changeset_id: same(),
            attachment_id: same(),
            attachment_name: String::new(),
            attachment_text: String::new(),
            doc_id: same(),
            doc_title: String::new(),
            doc_text: String::new(),
            claimed_node: UNKNOWN_NODE.to_string(),
            version_id: same(),
            other_version_id: same(),
            merged_changeset_id: same(),
            conflict_id: same(),
            thread_id: same(),
            thread_question: String::new(),
            post_id: same(),
            post_summary: String::new(),
            decision_id: same(),
            decision_direction: String::new(),
            comment_id: same(),
            comment_node: UNKNOWN_NODE.to_string(),
            comment_text: String::new(),
            feature_id: same(),
            feature_name: String::new(),
        }
    }

    /// Everything about the workspace that must never appear in an answer to
    /// another workspace.
    fn markers(&self) -> Vec<&str> {
        [
            &self.id,
            &self.email,
            &self.key_id,
            &self.key_prefix,
            &self.project_id,
            &self.project_name,
            &self.other_project_id,
            &self.changeset_id,
            &self.rejected_changeset_id,
            &self.attachment_id,
            &self.attachment_name,
            &self.attachment_text,
            &self.doc_id,
            &self.doc_title,
            &self.doc_text,
            &self.claimed_node,
            &self.version_id,
            &self.other_version_id,
            &self.merged_changeset_id,
            &self.conflict_id,
            &self.thread_id,
            &self.thread_question,
            &self.post_id,
            &self.post_summary,
            &self.decision_id,
            &self.decision_direction,
            &self.comment_id,
            &self.comment_text,
            &self.feature_id,
            &self.feature_name,
        ]
        .into_iter()
        .map(String::as_str)
        .filter(|m| !m.is_empty())
        .collect()
    }
}

struct Fixture {
    a: Side,
    b: Side,
    server: TestServer,
}

fn session_token(name: &str) -> String {
    format!("us018-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
}

fn fixture() -> &'static Fixture {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        block(async {
            let mut admin = PgConnection::connect_with(&admin_options())
                .await
                .unwrap_or_else(|e| panic!("cannot connect to Postgres as the admin role: {e}"));
            sqlx::query(&format!("DROP DATABASE IF EXISTS {TEST_DB} WITH (FORCE)"))
                .execute(&mut admin)
                .await
                .expect("drop the scratch database");
            sqlx::query(&format!("CREATE DATABASE {TEST_DB}"))
                .execute(&mut admin)
                .await
                .expect("create the scratch database");
            omni_api::migrate(&config(), 30)
                .await
                .expect("apply the migrations to the scratch database");
            if !bucket().exists().await.unwrap_or(false) {
                Bucket::create_with_path_style(
                    TEST_BUCKET,
                    s3_region(),
                    s3_credentials(),
                    BucketConfiguration::default(),
                )
                .await
                .unwrap_or_else(|e| panic!("cannot create the scratch bucket: {e}"));
            }
        });
        let workspaces = block(async {
            let mut conn = admin_connection().await;
            let mut text = async |sql: &str, binds: &[&str]| -> String {
                let mut query = sqlx::query_scalar::<_, String>(sql);
                for bind in binds {
                    query = query.bind(*bind);
                }
                query.fetch_one(&mut conn).await.expect("seed")
            };
            let mut found = Vec::new();
            for (name, workspace_name) in [("alice", "Alpha"), ("bob", "Beta")] {
                let workspace = text(
                    "insert into workspaces (name) values ($1) returning id::text",
                    &[workspace_name],
                )
                .await;
                let user = text(
                    "insert into users (email) values ($1) returning id::text",
                    &[&format!("{name}@example.test")],
                )
                .await;
                text(
                    "insert into sessions (user_id, token_hash, expires_at) \
                     values ($1::uuid, sha256(convert_to($2, 'UTF8')), now() + interval '1 day') \
                     returning id::text",
                    &[&user, &session_token(name)],
                )
                .await;
                text(
                    "insert into memberships (workspace_id, user_id) \
                     values ($1::uuid, $2::uuid) returning id::text",
                    &[&workspace, &user],
                )
                .await;
                found.push((name, workspace));
            }
            found
        });
        let server = TestServer::start(&config());
        let client = &server.client;
        let mut sides = workspaces.into_iter().map(|(name, workspace)| {
            let token = session_token(name);
            let human = Auth::Session(&token);
            let keys_path = format!("/api/v1/w/{workspace}/api-keys");
            let minted = must(
                call(client, "POST", &keys_path, &human, Some("{}")),
                201,
                "mint a key",
            );
            let key = minted["secret"].as_str().expect("a secret").to_string();
            let spare = must(
                call(client, "POST", &keys_path, &human, Some("{}")),
                201,
                "mint a spare key",
            );
            let agent = Auth::Key(&key);
            let project = |label: &str| -> (String, String) {
                let project_name = unique(&format!("{name}-{label}"));
                let body = json!({ "name": project_name }).to_string();
                let created = must(
                    call(
                        client,
                        "POST",
                        &format!("/api/v1/w/{workspace}/projects"),
                        &agent,
                        Some(&body),
                    ),
                    201,
                    "create a project",
                );
                let id = created["project"]["id"].as_str().expect("an id");
                (id.to_string(), project_name)
            };
            let (project_id, project_name) = project("roadmap");
            let (other_project_id, _) = project("backlog");
            let changesets = format!("/api/v1/w/{workspace}/projects/{project_id}/changesets");
            let changeset = || -> String {
                let body = json!({ "title": unique(&format!("{name}-change")) }).to_string();
                let created = must(
                    call(client, "POST", &changesets, &agent, Some(&body)),
                    201,
                    "create a change set",
                );
                created["changeset"]["id"]
                    .as_str()
                    .expect("an id")
                    .to_string()
            };
            let changeset_id = changeset();
            let rejected_changeset_id = changeset();
            let path = format!("{changesets}/{rejected_changeset_id}");
            must(
                call(client, "POST", &format!("{path}/submit"), &agent, None),
                200,
                "submit",
            );
            must(
                call(
                    client,
                    "POST",
                    &format!("{path}/reject"),
                    &human,
                    Some(r#"{"reason":"seeded"}"#),
                ),
                200,
                "reject",
            );
            // US-059: a feature of the project, so the features table has rows
            // of the workspace. Seeded as soon as the registry has the feature
            // routes.
            let (feature_id, feature_name) = if has_feature_routes() {
                let feature_name = unique(&format!("{name}-feature"));
                let body = json!({ "name": feature_name }).to_string();
                let created = must(
                    call(
                        client,
                        "POST",
                        &format!("/api/v1/w/{workspace}/projects/{project_id}/features"),
                        &agent,
                        Some(&body),
                    ),
                    201,
                    "create a feature",
                );
                let id = created["feature"]["id"].as_str().expect("an id");
                (id.to_string(), feature_name)
            } else {
                (String::new(), String::new())
            };
            // US-102: a doc at the app level and one in the feature, made
            // without a change set, so the log of the writes to docs has rows
            // of the workspace. Seeded as soon as the registry has the
            // scope-level doc routes.
            if has_live_doc_routes() && !feature_id.is_empty() {
                let project_path = format!("/api/v1/w/{workspace}/projects/{project_id}");
                for scope in [String::new(), format!("/features/{feature_id}")] {
                    let body = json!({
                        "kind": "html",
                        "pillar": "spec",
                        "title": unique(&format!("{name}-live-doc"))
                    })
                    .to_string();
                    must(
                        call(
                            client,
                            "POST",
                            &format!("{project_path}{scope}/docs"),
                            &agent,
                            Some(&body),
                        ),
                        201,
                        "create a doc without a change set",
                    );
                }
            }
            // US-019: a file of the workspace's own. The attachment routes are
            // seeded as soon as the registry has them; the table behind them
            // would otherwise make the checks below fail on a tenant table
            // without rows.
            let (attachment_id, attachment_name, attachment_text) = if has_attachment_routes() {
                let file_name = format!("{name}-notes.txt");
                let text = unique(&format!("{name}-file-content"));
                let uploaded =
                    upload_attachment(client, &workspace, &key, text.as_bytes(), &file_name);
                let uploaded = must(uploaded, 201, "upload a file");
                let id = uploaded["attachment"]["id"].as_str().expect("an id");
                (id.to_string(), file_name, text)
            } else {
                (String::new(), String::new(), String::new())
            };
            // US-022: a doc started in the open change set, with one applied
            // operation, so the doc, draft and log tables have rows of the
            // workspace. Seeded as soon as the registry has the doc routes.
            let (doc_id, doc_title, doc_text, claimed_node) = if has_doc_routes() {
                let doc_title = unique(&format!("{name}-doc"));
                let body =
                    json!({ "kind": "html", "pillar": "spec", "title": doc_title }).to_string();
                let created = must(
                    call(
                        client,
                        "POST",
                        &format!("{changesets}/{changeset_id}/docs"),
                        &agent,
                        Some(&body),
                    ),
                    201,
                    "create a doc",
                );
                let doc_id = created["doc"]["id"].as_str().expect("an id").to_string();
                let doc_text = unique(&format!("{name}-doc-content"));
                let ops = json!({ "ops": [{
                    "op": "insert",
                    "parent": null,
                    "position": { "after": null },
                    "content": format!("<p>{doc_text}</p>")
                }] })
                .to_string();
                must(
                    call(
                        client,
                        "POST",
                        &format!("{changesets}/{changeset_id}/docs/{doc_id}/ops"),
                        &agent,
                        Some(&ops),
                    ),
                    200,
                    "edit the doc's draft",
                );
                // US-023: a section of the draft that the agent claims, so
                // whatever holds claims has rows of the workspace. Seeded as
                // soon as the registry has the claim routes.
                let claimed_node = if has_claim_routes() {
                    let ops = json!({ "ops": [{
                        "op": "insert",
                        "parent": null,
                        "position": { "after": null },
                        "content": "<section><h2>Claimed</h2></section>"
                    }] })
                    .to_string();
                    let inserted = must(
                        call(
                            client,
                            "POST",
                            &format!("{changesets}/{changeset_id}/docs/{doc_id}/ops"),
                            &agent,
                            Some(&ops),
                        ),
                        200,
                        "add a section to the doc's draft",
                    );
                    let section = inserted["results"][0]["node"]
                        .as_str()
                        .expect("the new section");
                    must(
                        call(
                            client,
                            "POST",
                            &format!("{changesets}/{changeset_id}/claims"),
                            &agent,
                            Some(&json!({ "node": section }).to_string()),
                        ),
                        201,
                        "claim the section",
                    );
                    section.to_string()
                } else {
                    String::new()
                };
                (doc_id, doc_title, doc_text, claimed_node)
            } else {
                (String::new(), String::new(), String::new(), String::new())
            };
            // US-024: the draft saved as a version, edited and saved again, so
            // the version and node index tables have rows of the workspace and
            // the diff route has two versions to compare. Seeded as soon as the
            // registry has the version routes.
            let (version_id, other_version_id) = if has_version_routes() {
                let draft = format!("{changesets}/{changeset_id}/docs/{doc_id}");
                let save = || -> String {
                    let saved = must(
                        call(
                            client,
                            "POST",
                            &format!("{draft}/versions"),
                            &agent,
                            Some("{}"),
                        ),
                        201,
                        "save a version of the doc",
                    );
                    saved["version"]["id"]
                        .as_str()
                        .expect("a version id")
                        .to_string()
                };
                let version_id = save();
                let ops = json!({ "ops": [{
                    "op": "insert",
                    "parent": null,
                    "position": { "after": null },
                    "content": format!("<p>{}</p>", unique(&format!("{name}-later")))
                }] })
                .to_string();
                must(
                    call(client, "POST", &format!("{draft}/ops"), &agent, Some(&ops)),
                    200,
                    "edit the doc's draft again",
                );
                (version_id, save())
            } else {
                (String::new(), String::new())
            };
            // US-025: a change set whose one doc was saved, submitted, approved
            // by the human and merged, so whatever records approvals has rows
            // of the workspace. Seeded as soon as the registry has the approve
            // route.
            let merged_changeset_id = if has_approve_route() {
                let merged = changeset();
                let path = format!("{changesets}/{merged}");
                let body = json!({ "kind": "html", "pillar": "spec", "title": unique(&format!("{name}-merged-doc")) })
                    .to_string();
                let created = must(
                    call(client, "POST", &format!("{path}/docs"), &agent, Some(&body)),
                    201,
                    "create a doc to merge",
                );
                let doc = created["doc"]["id"].as_str().expect("an id");
                let ops = json!({ "ops": [{
                    "op": "insert",
                    "parent": null,
                    "position": { "after": null },
                    "content": format!("<p>{}</p>", unique(&format!("{name}-merged")))
                }] })
                .to_string();
                let draft = format!("{path}/docs/{doc}");
                must(
                    call(client, "POST", &format!("{draft}/ops"), &agent, Some(&ops)),
                    200,
                    "edit the doc to merge",
                );
                must(
                    call(
                        client,
                        "POST",
                        &format!("{draft}/versions"),
                        &agent,
                        Some("{}"),
                    ),
                    201,
                    "save the doc to merge",
                );
                must(
                    call(client, "POST", &format!("{path}/submit"), &agent, None),
                    200,
                    "submit the change set to merge",
                );
                let approved = must(
                    call(
                        client,
                        "POST",
                        &format!("{path}/approve"),
                        &human,
                        Some("{}"),
                    ),
                    200,
                    "approve the change set",
                );
                assert_eq!(approved["changeset"]["status"], "merged", "{approved}");
                merged
            } else {
                String::new()
            };
            // US-026: the open change set is approved and rebased onto a
            // mainline that moved on, with a conflict, so whatever records
            // conflicts has rows of the workspace and the conflict routes have
            // a conflict of `{changeset_id}`. Seeded as soon as the registry
            // has the rebase route.
            let conflict_id = if has_rebase_route() {
                seed_conflict(client, &changesets, &agent, &human, &changeset_id, name)
            } else {
                String::new()
            };
            // US-033: a thread of the open change set, moved on to a sealed
            // second phase, with a post, so whatever holds threads, phases and
            // posts has rows of the workspace. Seeded as soon as the registry
            // has the thread routes.
            let (thread_id, thread_question, post_id, post_summary, decision_id, decision_direction) = if has_thread_routes() {
                let question = unique(&format!("{name}-question"));
                let body = json!({ "question": question, "criteria": [unique(&format!("{name}-criterion"))] })
                    .to_string();
                let created = must(
                    call(
                        client,
                        "POST",
                        &format!("{changesets}/{changeset_id}/threads"),
                        &agent,
                        Some(&body),
                    ),
                    201,
                    "start a thread",
                );
                let thread = created["thread"]["id"].as_str().expect("an id");
                let threads = format!("/api/v1/w/{workspace}/projects/{project_id}/threads");
                must(
                    call(
                        client,
                        "POST",
                        &format!("{threads}/{thread}/phases"),
                        &agent,
                        Some(r#"{"sealed":true}"#),
                    ),
                    201,
                    "start the thread's second phase",
                );
                let post_summary = unique(&format!("{name}-summary"));
                let body = json!({
                    "type": "position",
                    "summary": post_summary,
                    "body": unique(&format!("{name}-post"))
                })
                .to_string();
                let posted = must(
                    call(
                        client,
                        "POST",
                        &format!("{threads}/{thread}/posts"),
                        &agent,
                        Some(&body),
                    ),
                    201,
                    "post in the thread",
                );
                let post_id = posted["post"]["id"].as_str().expect("an id").to_string();
                // US-034: a directive of the human in the thread, which the
                // agent is given with its next request in the change set, so
                // whatever records that has rows of the workspace.
                if has_directive_route() {
                    let body = json!({
                        "summary": unique(&format!("{name}-directive")),
                        "body": unique(&format!("{name}-directive-body"))
                    })
                    .to_string();
                    must(
                        call(
                            client,
                            "POST",
                            &format!("{threads}/{thread}/directives"),
                            &human,
                            Some(&body),
                        ),
                        201,
                        "the human posts a directive",
                    );
                    let shown = must(
                        call(
                            client,
                            "GET",
                            &format!("{changesets}/{changeset_id}"),
                            &agent,
                            None,
                        ),
                        200,
                        "show the change set to the agent",
                    );
                    assert!(
                        shown["directives"].as_array().is_some_and(|d| d.len() == 1),
                        "the agent is given the directive: {shown}"
                    );
                }
                // US-037: a decision for the thread, relying on the agent's
                // post, and the open change set linked to it, so whatever
                // holds decisions and their links has rows of the workspace.
                // Seeded as soon as the registry has the decision routes.
                let (decision_id, decision_direction) = if has_decision_routes() {
                    let direction = unique(&format!("{name}-direction"));
                    let body = json!({
                        "direction": direction,
                        "chosen": "Split hero",
                        "criteria": ["Readable at 320 px"],
                        "options": [
                            {
                                "name": "Split hero",
                                "scores": { "Readable at 320 px": 8 },
                                "lost_because": null
                            },
                            {
                                "name": "Carousel",
                                "scores": { "Readable at 320 px": 4 },
                                "lost_because": "It hides two of the three messages."
                            }
                        ],
                        "assumptions": [],
                        "open_questions": [],
                        "dissent": [],
                        "posts": [post_id]
                    })
                    .to_string();
                    let proposed = must(
                        call(
                            client,
                            "POST",
                            &format!("{threads}/{thread}/decisions"),
                            &agent,
                            Some(&body),
                        ),
                        201,
                        "propose a decision",
                    );
                    let decision = proposed["decision"]["id"]
                        .as_str()
                        .expect("an id")
                        .to_string();
                    must(
                        call(
                            client,
                            "POST",
                            &format!("{changesets}/{changeset_id}/decisions/{decision}"),
                            &agent,
                            None,
                        ),
                        200,
                        "link the change set to the decision",
                    );
                    (decision, direction)
                } else {
                    (String::new(), String::new())
                };
                (
                    thread.to_string(),
                    question,
                    post_id,
                    post_summary,
                    decision_id,
                    decision_direction,
                )
            } else {
                (
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                )
            };
            // US-038: the human's comment on the paragraph of the first saved
            // version, the agent's reply to it and its mark that the second
            // version addressed it, so whatever holds comments and replies
            // has rows of the workspace. Seeded as soon as the registry has
            // the comment routes.
            let (comment_id, comment_node, comment_text) = if has_comment_routes()
                && has_version_routes()
            {
                let base = format!("/api/v1/w/{workspace}/projects/{project_id}");
                let version = must(
                    call(
                        client,
                        "GET",
                        &format!("{base}/versions/{version_id}"),
                        &agent,
                        None,
                    ),
                    200,
                    "read the first saved version",
                );
                let node = version["version"]["nodes"]
                    .as_array()
                    .and_then(|nodes| nodes.iter().find(|n| n["type"] == "paragraph"))
                    .and_then(|n| n["id"].as_str())
                    .unwrap_or_else(|| panic!("the version has a paragraph: {version}"))
                    .to_string();
                let text = unique(&format!("{name}-comment"));
                let body = json!({ "node": node, "text": text }).to_string();
                let made = must(
                    call(
                        client,
                        "POST",
                        &format!("{base}/versions/{version_id}/comments"),
                        &human,
                        Some(&body),
                    ),
                    201,
                    "the human comments on the paragraph",
                );
                let comment = made["comment"]["id"]
                    .as_str()
                    .expect("an id")
                    .to_string();
                let body = json!({ "text": unique(&format!("{name}-reply")) }).to_string();
                must(
                    call(
                        client,
                        "POST",
                        &format!("{base}/comments/{comment}/replies"),
                        &agent,
                        Some(&body),
                    ),
                    201,
                    "the agent replies to the comment",
                );
                // Comments have no status after US-106; mark it addressed
                // only while the route exists.
                if has_route(
                    "POST",
                    "/api/v1/w/{workspace_id}/projects/{project_id}/comments/{comment_id}/addressed",
                ) {
                    let body = json!({ "version_id": other_version_id }).to_string();
                    must(
                        call(
                            client,
                            "POST",
                            &format!("{base}/comments/{comment}/addressed"),
                            &agent,
                            Some(&body),
                        ),
                        200,
                        "the agent marks the comment addressed",
                    );
                }
                (comment, node, text)
            } else {
                (String::new(), String::new(), String::new())
            };
            Side {
                id: workspace,
                email: format!("{name}@example.test"),
                token: token.clone(),
                key: key.clone(),
                key_id: spare["key"]["id"].as_str().expect("an id").to_string(),
                key_prefix: spare["key"]["prefix"]
                    .as_str()
                    .expect("a prefix")
                    .to_string(),
                project_id,
                project_name,
                other_project_id,
                changeset_id,
                rejected_changeset_id,
                attachment_id,
                attachment_name,
                attachment_text,
                doc_id,
                doc_title,
                doc_text,
                claimed_node,
                version_id,
                other_version_id,
                merged_changeset_id,
                conflict_id,
                thread_id,
                thread_question,
                post_id,
                post_summary,
                decision_id,
                decision_direction,
                comment_id,
                comment_node,
                comment_text,
                feature_id,
                feature_name,
            }
        });
        let a = sides.next().expect("workspace A");
        let b = sides.next().expect("workspace B");
        Fixture { a, b, server }
    })
}

fn ready() -> (&'static Fixture, &'static Client) {
    let fixture = fixture();
    (fixture, &fixture.server.client)
}

/// `template` with its placeholders filled: `{workspace_id}` from `workspace`,
/// the IDs of records from `record`.
fn fill(template: &str, workspace: &Side, record: &Side) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let end = start
            + rest[start..]
                .find('}')
                .unwrap_or_else(|| panic!("unclosed placeholder in {template}"));
        out.push_str(match &rest[start + 1..end] {
            "workspace_id" => &workspace.id,
            "project_id" => &record.project_id,
            "changeset_id" => &record.changeset_id,
            "key_id" => &record.key_id,
            "attachment_id" => &record.attachment_id,
            "doc_id" => &record.doc_id,
            "version_id" => &record.version_id,
            "other_version_id" => &record.other_version_id,
            "conflict_id" => &record.conflict_id,
            "thread_id" => &record.thread_id,
            "post_id" => &record.post_id,
            "decision_id" => &record.decision_id,
            "comment_id" => &record.comment_id,
            "feature_id" => &record.feature_id,
            // US-030: an attack on a board is refused at the workspace or the
            // project, before any board is looked up, so a node ID that no
            // board has stands in for one.
            "board_id" => UNKNOWN_NODE,
            // US-111: every doc's log starts with its creation, seq 1.
            "seq" => "1",
            other => panic!(
                "{template} has the placeholder {{{other}}}, for which the suite has no seeded \
                 ID: add it to Side and fill() in tests/isolation.rs"
            ),
        });
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Whether the path names a record besides the workspace.
/// The body of `case` for calls about the records of `record`: its
/// `<claimed_node>` is the section that `record`'s workspace claimed, its
/// `<comment_node>` the node of `record`'s seeded comment and its
/// `<other_version_id>` `record`'s second saved version.
fn body_of(case: &Case, record: &Side) -> Option<String> {
    case.body.map(|body| {
        body.replace("<claimed_node>", &record.claimed_node)
            .replace("<comment_node>", &record.comment_node)
            .replace("<other_version_id>", &record.other_version_id)
    })
}

fn names_a_record(template: &str) -> bool {
    template.matches('{').count() > 1
}

/// The credentials of `side` that the route accepts, as the route's access
/// says: an API key for an agent route, a session for a human one, either for both.
fn credentials(access: Access, side: &Side) -> Vec<Auth<'_>> {
    match access {
        Access::Agent => vec![Auth::Key(&side.key)],
        Access::Human => vec![Auth::Session(&side.token)],
        Access::Both => vec![Auth::Key(&side.key), Auth::Session(&side.token)],
    }
}

fn assert_not_found_without_data(response: &Response, victim: &Side, what: &str) {
    assert_eq!(response.status, 404, "{what}: {}", response.text());
    assert_eq!(
        response.json(),
        json!({ "error": "not_found" }),
        "{what}: the answer carries the code and nothing else"
    );
    for marker in victim.markers() {
        assert!(
            !response.text().contains(marker),
            "{what}: the answer leaks {marker}: {}",
            response.text()
        );
    }
}

// ---------------------------------------------------------------------------
// Database snapshots
// ---------------------------------------------------------------------------

fn scope_column(table: &str) -> &'static str {
    if table == "workspaces" {
        "id"
    } else {
        "workspace_id"
    }
}

/// Every tenant table: `workspaces` and each table with a `workspace_id` column.
async fn tenant_tables(conn: &mut PgConnection) -> Vec<String> {
    let mut tables: Vec<String> = sqlx::query_scalar(
        "select distinct table_name::text from information_schema.columns \
         where table_schema = 'public' and column_name = 'workspace_id' \
         and table_name <> '_sqlx_migrations'",
    )
    .fetch_all(&mut *conn)
    .await
    .expect("list the tenant tables");
    tables.push("workspaces".to_string());
    tables.sort();
    tables.dedup();
    tables
}

async fn rows_of(conn: &mut PgConnection, table: &str, workspace: &str) -> i64 {
    let column = scope_column(table);
    sqlx::query_scalar(&format!(
        "select count(*) from {table} where {column} = '{workspace}'::uuid"
    ))
    .fetch_one(&mut *conn)
    .await
    .unwrap_or_else(|e| panic!("count the rows of {table}: {e}"))
}

/// A digest of every row of the workspace in every tenant table, so a test can
/// tell that an attack changed nothing.
fn snapshot(workspace: &str) -> Vec<(String, String)> {
    block(async {
        let mut conn = admin_connection().await;
        let mut found = Vec::new();
        for table in tenant_tables(&mut conn).await {
            let column = scope_column(&table);
            let digest: String = sqlx::query_scalar(&format!(
                "select md5(coalesce(string_agg(t::text, '|' order by t::text), '')) \
                 from {table} t where t.{column} = '{workspace}'::uuid"
            ))
            .fetch_one(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("digest {table}: {e}"));
            found.push((table, digest));
        }
        found
    })
}

// ---------------------------------------------------------------------------
// Coverage of the routes
// ---------------------------------------------------------------------------

#[test]
fn every_route_in_the_registry_has_suite_coverage() {
    let routes = registry();
    assert!(!routes.is_empty());
    let missing = uncovered(&routes, CASES);
    assert!(
        missing.is_empty(),
        "routes without suite coverage, add a case to CASES in tests/isolation.rs: {missing:?}"
    );
    let gone = stale(&routes, CASES);
    assert!(
        gone.is_empty(),
        "cases for routes that the registry no longer has: {gone:?}"
    );
    for entry in &routes {
        let name = route_name(&entry.method, &entry.path);
        match case_for(entry).kind {
            Kind::Workspace => assert!(
                entry.path.contains("{workspace_id}"),
                "{name} is a Workspace case without {{workspace_id}} in its path"
            ),
            Kind::Global(reason) => {
                assert!(!reason.is_empty(), "{name} is Global without a reason");
                assert!(
                    !entry.path.contains("{workspace_id}"),
                    "{name} has a workspace in its path, so the suite must attack it"
                );
            }
        }
    }
}

#[test]
fn a_route_without_suite_coverage_is_reported() {
    let mut routes = registry();
    assert!(
        uncovered(&routes, CASES).is_empty(),
        "the real registry is covered"
    );
    routes.push(RouteEntry {
        method: "GET".to_string(),
        path: "/api/v1/w/{workspace_id}/projects/{project_id}/documents".to_string(),
        access: Access::Both,
    });
    routes.push(RouteEntry {
        method: "DELETE".to_string(),
        path: "/api/v1/w/{workspace_id}/projects".to_string(),
        access: Access::Agent,
    });
    assert_eq!(
        uncovered(&routes, CASES),
        [
            "GET /api/v1/w/{workspace_id}/projects/{project_id}/documents",
            "DELETE /api/v1/w/{workspace_id}/projects"
        ],
        "a new route, and a new method on a covered path, are both uncovered"
    );
    let without_one = &CASES[1..];
    assert_eq!(
        uncovered(&registry(), without_one),
        ["GET /api/health"],
        "a case taken away uncovers its route"
    );
}

#[test]
fn every_websocket_route_has_suite_coverage() {
    let routes = websocket_registry();
    let paths: BTreeSet<&str> = routes.iter().map(|r| r.path.as_str()).collect();
    let covered: BTreeSet<&str> = WEBSOCKET_PATHS.iter().copied().collect();
    assert_eq!(
        paths, covered,
        "the WebSocket routes of websocket_registry() against the paths this suite attacks \
         (WEBSOCKET_PATHS in tests/isolation.rs)"
    );
    for route in &routes {
        assert_eq!(route.method, "GET", "{}", route.path);
        assert!(
            route.path.starts_with("/ws/") && route.path.contains("{workspace_id}"),
            "a WebSocket route lives under /ws/ and names its workspace: {}",
            route.path
        );
    }
}

/// The path literals of every `.route("...")` call in the API's source.
fn served_paths() -> BTreeSet<String> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = BTreeSet::new();
    for entry in std::fs::read_dir(&source).expect("read src/") {
        let path = entry.expect("a directory entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read a source file");
        let mut rest = text.as_str();
        while let Some(at) = rest.find(".route(") {
            rest = rest[at + ".route(".len()..].trim_start();
            if let Some(literal) = rest.strip_prefix('"')
                && let Some(end) = literal.find('"')
            {
                found.insert(literal[..end].to_string());
            }
        }
    }
    found
}

#[test]
fn every_path_that_the_router_serves_is_in_a_route_registry() {
    let served = served_paths();
    assert!(
        served.contains("/api/health") && served.contains("/api/v1/auth/login"),
        "the scan of src/ found no routes, so it proves nothing: {served:?}"
    );
    let registered: BTreeSet<String> = registry()
        .into_iter()
        .chain(websocket_registry())
        .map(|r| r.path)
        .collect();
    let hidden: Vec<&String> = served.difference(&registered).collect();
    assert!(
        hidden.is_empty(),
        "routes that the router serves but no registry lists, so no suite attacks them \
         (add them to routes::registry() or routes::websocket_registry()): {hidden:?}"
    );
    let phantom: Vec<&String> = registered.difference(&served).collect();
    assert!(
        phantom.is_empty(),
        "registry entries that the router does not serve: {phantom:?}"
    );
}

#[test]
fn the_suite_runs_in_make_test() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let read = |file: &str| -> String {
        std::fs::read_to_string(root.join(file)).unwrap_or_else(|e| panic!("read {file}: {e}"))
    };

    // `make test` runs the test container.
    let makefile = read("Makefile");
    let target = makefile
        .split("\ntest:")
        .nth(1)
        .and_then(|rest| rest.split("\n\n").next())
        .expect("the test target of the Makefile");
    assert!(
        target.contains("docker compose run --rm test"),
        "make test must run the test container: {target}"
    );

    // The container runs all of omni-api's tests, not a chosen few.
    let dockerfile = read("docker/test.Dockerfile");
    let run = dockerfile
        .split("&&")
        .find(|part| part.contains("-p omni-api"))
        .expect("the test container runs omni-api's tests");
    assert!(
        run.contains("--features stack-tests") && !run.contains("--test "),
        "the container must run every omni-api test target: {run}"
    );

    // The suite needs Postgres, so its target is built with the stack-tests
    // feature only, which keeps `cargo test --workspace` on the host green.
    let manifest = read("crates/omni-api/Cargo.toml");
    let target = manifest
        .split("[[test]]")
        .find(|section| section.contains("name = \"isolation\""))
        .expect("Cargo.toml registers the isolation test target");
    assert!(
        target
            .lines()
            .any(|l| l.trim() == "required-features = [\"stack-tests\"]"),
        "the isolation target needs required-features = [\"stack-tests\"]: {target}"
    );
}

// ---------------------------------------------------------------------------
// Workspace A against workspace B, over HTTP
// ---------------------------------------------------------------------------

#[test]
fn credentials_of_workspace_a_get_404_and_no_data_for_the_workspace_and_ids_of_b() {
    let (f, client) = ready();
    let _guard = serial();
    let before = snapshot(&f.b.id);
    let mut attacks = 0;
    for entry in registry() {
        let case = case_for(&entry);
        if !matches!(case.kind, Kind::Workspace) {
            continue;
        }
        let path = fill(&entry.path, &f.b, &f.b);
        let mut callers = credentials(entry.access, &f.a);
        if entry.access == Access::Agent {
            // The workspace is checked before the caller's authority (FR-5): a
            // session of A on an agent route of B is a stranger first.
            callers.push(Auth::Session(&f.a.token));
        }
        for auth in callers {
            let what = format!(
                "{} with {} of workspace A against B",
                route_name(&entry.method, &path),
                auth.label()
            );
            let body = body_of(case, &f.b);
            let response = call(client, &entry.method, &path, &auth, body.as_deref());
            assert_not_found_without_data(&response, &f.b, &what);
            attacks += 1;
        }
    }
    assert!(attacks >= 20, "only {attacks} attacks ran");
    assert_eq!(
        snapshot(&f.b.id),
        before,
        "an attack on workspace B changed its rows"
    );
}

#[test]
fn ids_of_workspace_b_under_the_workspace_of_a_get_the_answer_for_an_id_that_does_not_exist() {
    let (f, client) = ready();
    let _guard = serial();
    let before = snapshot(&f.b.id);
    let nobody = Side::stranger(UNKNOWN_ID);
    let mut attacks = 0;
    for entry in registry() {
        let case = case_for(&entry);
        if !matches!(case.kind, Kind::Workspace) || !names_a_record(&entry.path) {
            continue;
        }
        let stolen = fill(&entry.path, &f.a, &f.b);
        let missing = fill(&entry.path, &f.a, &nobody);
        for auth in credentials(entry.access, &f.a) {
            let what = format!(
                "{} with {} of workspace A and B's IDs",
                route_name(&entry.method, &stolen),
                auth.label()
            );
            let body = body_of(case, &f.b);
            let response = call(client, &entry.method, &stolen, &auth, body.as_deref());
            assert_not_found_without_data(&response, &f.b, &what);
            let body = body_of(case, &nobody);
            let unknown = call(client, &entry.method, &missing, &auth, body.as_deref());
            assert_eq!(
                (response.status, response.body.clone()),
                (unknown.status, unknown.body.clone()),
                "{what}: an ID of another workspace is answered like one that does not exist"
            );
            attacks += 1;
        }
    }
    assert!(attacks >= 10, "only {attacks} attacks ran");
    assert_eq!(
        snapshot(&f.b.id),
        before,
        "an attack on workspace B changed its rows"
    );
}

#[test]
fn the_same_attacks_work_the_other_way_round() {
    let (f, client) = ready();
    let _guard = serial();
    let before = snapshot(&f.a.id);
    for entry in registry() {
        let case = case_for(&entry);
        if !matches!(case.kind, Kind::Workspace) {
            continue;
        }
        let abroad = fill(&entry.path, &f.a, &f.a);
        let stolen = fill(&entry.path, &f.b, &f.a);
        for auth in credentials(entry.access, &f.b) {
            let what = format!(
                "{} with {} of workspace B against A",
                route_name(&entry.method, &abroad),
                auth.label()
            );
            let body = body_of(case, &f.a);
            let response = call(client, &entry.method, &abroad, &auth, body.as_deref());
            assert_not_found_without_data(&response, &f.a, &what);
            if names_a_record(&entry.path) {
                let response = call(client, &entry.method, &stolen, &auth, body.as_deref());
                assert_not_found_without_data(&response, &f.a, &what);
            }
        }
    }
    assert_eq!(
        snapshot(&f.a.id),
        before,
        "an attack on workspace A changed it"
    );
}

#[test]
fn the_lists_of_a_workspace_hold_nothing_of_the_other() {
    let (f, client) = ready();
    for (own, other) in [(&f.a, &f.b), (&f.b, &f.a)] {
        for entry in registry() {
            let case = case_for(&entry);
            if !matches!(case.kind, Kind::Workspace) || entry.method != "GET" {
                continue;
            }
            // US-030: the seeded doc is an HTML doc and fill() names no board,
            // so the board reads have a control of their own, against a
            // design of the workspace (the_board_reads_of_a_workspace_...).
            if entry.path.contains("/boards/") {
                continue;
            }
            let path = fill(&entry.path, own, own);
            for auth in credentials(entry.access, own) {
                let what = format!("{} with {}", route_name(&entry.method, &path), auth.label());
                let response = call(client, &entry.method, &path, &auth, None);
                assert_eq!(response.status, 200, "{what}: {}", response.text());
                for marker in other.markers() {
                    assert!(
                        !response.text().contains(marker),
                        "{what} shows {marker} of the other workspace: {}",
                        response.text()
                    );
                }
                if entry.path.contains("/attachments/") {
                    assert_eq!(
                        response.text(),
                        own.attachment_text,
                        "{what}: the control call returns the workspace's own file"
                    );
                } else if names_a_record(&entry.path)
                    || entry.path.ends_with("/projects")
                    || entry.path.ends_with("/api-keys")
                {
                    let text = response.text();
                    assert!(
                        text.contains(&own.project_id)
                            || text.contains(&own.changeset_id)
                            || text.contains(&own.key_id)
                            || text.contains(&own.doc_id),
                        "{what}: the control call shows none of its own records: {text}"
                    );
                }
            }
        }
    }
}

#[test]
fn the_calls_of_the_attacks_work_for_the_workspace_that_owns_the_records() {
    // The control: the same requests, with the same bodies, succeed for the
    // owner, so the 404s above come from isolation and not from a bad request.
    let (f, client) = ready();
    let _guard = serial();
    let key = Auth::Key(&f.a.key);
    let human = Auth::Session(&f.a.token);
    let path = |template: &str| fill(template, &f.a, &f.a);

    let projects = case_named("POST", "/api/v1/w/{workspace_id}/projects");
    let body = projects
        .body
        .map(|b| b.replace("intruder", &unique("owner")));
    let response = call(client, "POST", &path(projects.path), &key, body.as_deref());
    must(response, 201, "create a project");

    let changesets = case_named(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets",
    );
    let created = must(
        call(
            client,
            "POST",
            &path(changesets.path),
            &key,
            changesets.body,
        ),
        201,
        "create a change set",
    );
    let id = created["changeset"]["id"].as_str().expect("an id");
    let base = format!(
        "/api/v1/w/{}/projects/{}/changesets/{id}",
        f.a.id, f.a.project_id
    );
    let action = |name: &str, auth: &Auth| {
        let template = format!(
            "/api/v1/w/{{workspace_id}}/projects/{{project_id}}/changesets/{{changeset_id}}/{name}"
        );
        let case = case_named("POST", &template);
        call(client, "POST", &format!("{base}/{name}"), auth, case.body)
    };
    assert_eq!(action("submit", &key).status, 200);
    assert_eq!(action("reject", &human).status, 200);
    assert_eq!(action("reopen", &key).status, 200);
    assert_eq!(action("abandon", &key).status, 200);

    let keys = case_named("POST", "/api/v1/w/{workspace_id}/api-keys");
    must(
        call(client, "POST", &path(keys.path), &human, keys.body),
        201,
        "mint a key",
    );
    let spare = must(
        call(client, "POST", &path(keys.path), &human, keys.body),
        201,
        "mint a key to revoke",
    );
    let spare_id = spare["key"]["id"].as_str().expect("an id");
    let revoked = must(
        call(
            client,
            "POST",
            &format!("/api/v1/w/{}/api-keys/{spare_id}/revoke", f.a.id),
            &human,
            None,
        ),
        200,
        "revoke a key",
    );
    assert!(revoked["key"]["revoked_at"].is_string(), "{revoked}");

    // US-022: the doc and draft calls, with the bodies of their cases.
    let docs = case_named(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs",
    );
    let body = docs.body.map(|b| b.replace("intruder", &unique("owner")));
    let created = must(
        call(client, "POST", &path(docs.path), &key, body.as_deref()),
        201,
        "create a doc",
    );
    let doc_id = created["doc"]["id"].as_str().expect("an id");
    let draft = format!(
        "/api/v1/w/{}/projects/{}/changesets/{}/docs/{doc_id}",
        f.a.id, f.a.project_id, f.a.changeset_id
    );
    let open = case_named(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/open",
    );
    let opened = call(client, "POST", &format!("{draft}/open"), &key, open.body);
    assert!(
        matches!(opened.status, 200 | 201),
        "open the doc: {}",
        opened.text()
    );
    let ops = case_named(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/ops",
    );
    must(
        call(client, "POST", &format!("{draft}/ops"), &key, ops.body),
        200,
        "edit the draft",
    );
    let doc = format!(
        "/api/v1/w/{}/projects/{}/docs/{doc_id}",
        f.a.id, f.a.project_id
    );
    for auth in [&key, &human] {
        must(
            call(client, "GET", &draft, auth, None),
            200,
            "read the draft",
        );
        must(call(client, "GET", &doc, auth, None), 200, "show the doc");
        must(
            call(client, "GET", &format!("{doc}/html"), auth, None),
            200,
            "read mainline",
        );
    }
}

#[test]
fn every_workspace_route_needs_credentials() {
    let (f, client) = ready();
    for entry in registry() {
        let case = case_for(&entry);
        if !matches!(case.kind, Kind::Workspace) {
            continue;
        }
        for (workspace, record) in [(&f.a, &f.a), (&f.b, &f.b)] {
            let path = fill(&entry.path, workspace, record);
            let body = body_of(case, record);
            let response = call(client, &entry.method, &path, &Auth::Nobody, body.as_deref());
            assert_eq!(
                response.status,
                401,
                "{} without credentials: {}",
                route_name(&entry.method, &path),
                response.text()
            );
            assert_eq!(response.json(), json!({ "error": "unauthorized" }));
        }
    }
}

#[test]
fn the_workspace_of_a_request_is_the_one_in_the_path_never_one_in_the_body_query_or_header() {
    let (f, client) = ready();
    let _guard = serial();
    let name = unique("smuggled");
    let body = json!({ "name": name, "workspace_id": f.b.id }).to_string();
    let path = format!("/api/v1/w/{}/projects?workspace_id={}", f.a.id, f.b.id);
    let response = call_with(
        client,
        "POST",
        &path,
        &Auth::Key(&f.a.key),
        &[
            ("X-Workspace-Id", f.b.id.as_str()),
            ("X-Omni-Workspace", f.b.id.as_str()),
        ],
        Some(&body),
    );
    must(response, 201, "create a project");
    let owner: Vec<String> = block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar("select workspace_id::text from projects where name = $1")
            .bind(&name)
            .fetch_all(&mut conn)
            .await
            .expect("look the project up")
    });
    assert_eq!(
        owner,
        [f.a.id.clone()],
        "the project belongs to the path's workspace"
    );

    // The other way: a request to B's path, with A named elsewhere, stays B's
    // business, so A's key still gets 404.
    let path = format!("/api/v1/w/{}/projects?workspace_id={}", f.b.id, f.a.id);
    let response = call_with(
        client,
        "GET",
        &path,
        &Auth::Key(&f.a.key),
        &[("X-Workspace-Id", f.a.id.as_str())],
        None,
    );
    assert_not_found_without_data(&response, &f.b, "A's key on B's path naming A elsewhere");
}

// ---------------------------------------------------------------------------
// Attachments (US-019)
// ---------------------------------------------------------------------------

#[test]
fn the_attachment_routes_are_in_the_registry_and_the_suite_attacks_them() {
    let wanted = [
        (
            "POST",
            "/api/v1/w/{workspace_id}/attachments",
            Access::Agent,
        ),
        (
            "GET",
            "/api/v1/w/{workspace_id}/attachments/{attachment_id}",
            Access::Both,
        ),
    ];
    let routes = registry();
    for (method, path, access) in wanted {
        let entry = routes
            .iter()
            .find(|r| r.method == method && r.path == path)
            .unwrap_or_else(|| panic!("the registry has no {}", route_name(method, path)));
        assert_eq!(entry.access, access, "{}", route_name(method, path));
        assert!(
            matches!(case_for(entry).kind, Kind::Workspace),
            "{} is a workspace route that the suite attacks",
            route_name(method, path)
        );
    }

    let (f, client) = ready();
    let _guard = serial();
    for side in [&f.a, &f.b] {
        assert!(
            !side.attachment_id.is_empty(),
            "the suite seeded no attachment for {}",
            side.id
        );
    }
    let b_folder = format!("ws/{}/attachments/", f.b.id);
    let a_folder = format!("ws/{}/attachments/", f.a.id);
    assert!(
        keys_under(&b_folder).contains(&format!("{b_folder}{}", f.b.attachment_id)),
        "B's file is stored in B's folder"
    );
    assert!(
        keys_under(&a_folder).contains(&format!("{a_folder}{}", f.a.attachment_id)),
        "A's file is stored in A's folder"
    );

    let before = snapshot(&f.b.id);
    let objects = keys_under(&format!("ws/{}/", f.b.id));
    let read_b_file = |workspace: &Side, auth: &Auth| {
        let path = format!(
            "/api/v1/w/{}/attachments/{}",
            workspace.id, f.b.attachment_id
        );
        call(client, "GET", &path, auth, None)
    };
    // A's credentials read B's file neither under B's workspace nor under A's.
    for auth in [
        Auth::Key(&f.a.key),
        Auth::Session(&f.a.token),
        Auth::KeyAndSession(&f.a.key, &f.a.token),
    ] {
        for workspace in [&f.b, &f.a] {
            let what = format!(
                "B's file under workspace {} with {}",
                workspace.id,
                auth.label()
            );
            let response = read_b_file(workspace, &auth);
            assert_not_found_without_data(&response, &f.b, &what);
        }
    }
    // A's credentials cannot put a file into B's workspace.
    let intruder = upload_attachment(client, &f.b.id, &f.a.key, b"intruder", "intruder.txt");
    assert_not_found_without_data(&intruder, &f.b, "A's key uploading into B");
    let intruder = call(
        client,
        "POST",
        &format!("/api/v1/w/{}/attachments", f.b.id),
        &Auth::Session(&f.a.token),
        Some("intruder"),
    );
    assert_not_found_without_data(&intruder, &f.b, "A's session uploading into B");
    assert_eq!(snapshot(&f.b.id), before, "an attack changed B's rows");
    assert_eq!(
        keys_under(&format!("ws/{}/", f.b.id)),
        objects,
        "an attack changed B's files"
    );

    // The control: the owner uploads and reads, and the file lands in its folder.
    let name = unique("owner") + ".txt";
    let uploaded = must(
        upload_attachment(client, &f.a.id, &f.a.key, name.as_bytes(), &name),
        201,
        "upload a file",
    );
    let id = uploaded["attachment"]["id"].as_str().expect("an id");
    assert!(keys_under(&a_folder).contains(&format!("{a_folder}{id}")));
    let path = format!("/api/v1/w/{}/attachments/{id}", f.a.id);
    for auth in [Auth::Key(&f.a.key), Auth::Session(&f.a.token)] {
        let response = call(client, "GET", &path, &auth, None);
        assert_eq!(response.status, 200, "read the file: {}", response.text());
        assert_eq!(response.text(), name, "{}", auth.label());
    }
}

// ---------------------------------------------------------------------------
// Versions (US-024)
// ---------------------------------------------------------------------------

#[test]
fn the_version_routes_are_in_the_registry_and_the_suite_attacks_them() {
    let routes = registry();
    for path in [
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/versions",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/remove",
        "/api/v1/w/{workspace_id}/projects/{project_id}/versions/{version_id}",
        "/api/v1/w/{workspace_id}/projects/{project_id}/versions/{version_id}/diff/{other_version_id}",
    ] {
        let entry = routes
            .iter()
            .find(|r| r.path == path)
            .unwrap_or_else(|| panic!("the registry has no route {path}"));
        assert!(
            matches!(case_for(entry).kind, Kind::Workspace),
            "{path} is a workspace route that the suite attacks"
        );
    }

    let (f, _) = ready();
    for side in [&f.a, &f.b] {
        assert!(
            !side.version_id.is_empty() && !side.other_version_id.is_empty(),
            "the suite seeded no versions for {}",
            side.id
        );
    }
}

#[test]
fn the_version_calls_of_the_attacks_work_for_the_workspace_that_owns_the_records() {
    // The control for the version routes: the owner's agent saves a version of
    // a doc of its own, reads it, compares it and removes the doc, with the
    // bodies of the cases.
    let (f, client) = ready();
    let _guard = serial();
    let key = Auth::Key(&f.a.key);
    let human = Auth::Session(&f.a.token);
    let path = |template: &str| fill(template, &f.a, &f.a);

    let docs = case_named(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs",
    );
    let body = docs.body.map(|b| b.replace("intruder", &unique("owner")));
    let created = must(
        call(client, "POST", &path(docs.path), &key, body.as_deref()),
        201,
        "create a doc",
    );
    let doc_id = created["doc"]["id"].as_str().expect("an id");
    let draft = format!(
        "/api/v1/w/{}/projects/{}/changesets/{}/docs/{doc_id}",
        f.a.id, f.a.project_id, f.a.changeset_id
    );
    let ops = case_named(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/ops",
    );
    must(
        call(client, "POST", &format!("{draft}/ops"), &key, ops.body),
        200,
        "edit the draft",
    );
    let save = case_named(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/versions",
    );
    let saved = must(
        call(
            client,
            "POST",
            &format!("{draft}/versions"),
            &key,
            save.body,
        ),
        201,
        "save a version",
    );
    let version = saved["version"]["id"].as_str().expect("an id");
    let versions = format!(
        "/api/v1/w/{}/projects/{}/versions/{version}",
        f.a.id, f.a.project_id
    );
    for auth in [&key, &human] {
        let read = must(
            call(client, "GET", &versions, auth, None),
            200,
            "read the version",
        );
        assert_eq!(read["version"]["doc_id"], doc_id, "{read}");
        must(
            call(
                client,
                "GET",
                &format!("{versions}/diff/{version}"),
                auth,
                None,
            ),
            200,
            "compare the version with itself",
        );
    }
    let remove = case_named(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/remove",
    );
    let removed = must(
        call(
            client,
            "POST",
            &format!("{draft}/remove"),
            &key,
            remove.body,
        ),
        201,
        "remove the doc",
    );
    assert_eq!(removed["version"]["removed"], true, "{removed}");
}

// ---------------------------------------------------------------------------
// Rebase and conflicts (US-026)
// ---------------------------------------------------------------------------

#[test]
fn the_rebase_and_conflict_routes_are_in_the_registry_and_the_suite_attacks_them() {
    for (method, path, access) in [
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/rebase",
            Access::Agent,
        ),
        (
            "GET",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/conflicts",
            Access::Both,
        ),
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/conflicts/{conflict_id}/resolve",
            Access::Agent,
        ),
    ] {
        let entry = registry()
            .into_iter()
            .find(|r| r.method == method && r.path == path)
            .unwrap_or_else(|| panic!("the registry has no route {method} {path}"));
        assert_eq!(entry.access, access, "{method} {path}");
        assert!(
            matches!(case_for(&entry).kind, Kind::Workspace),
            "{path} is a workspace route that the suite attacks"
        );
    }

    let (f, _) = ready();
    for side in [&f.a, &f.b] {
        assert!(
            !side.conflict_id.is_empty(),
            "the suite seeded no conflict for {}",
            side.id
        );
    }
}

#[test]
fn the_rebase_and_conflict_calls_of_the_attacks_work_for_the_workspace_that_owns_the_records() {
    // The control for the rebase, conflicts and resolve routes: the owner's
    // agent rebases a change set of its own workspace into a conflict, lists
    // it and resolves it, with the bodies of the cases.
    let (f, client) = ready();
    let _guard = serial();
    let key = Auth::Key(&f.a.key);
    let human = Auth::Session(&f.a.token);
    let changesets = format!(
        "/api/v1/w/{}/projects/{}/changesets",
        f.a.id, f.a.project_id
    );
    let body = json!({ "title": unique("owner-rebase") }).to_string();
    let created = must(
        call(client, "POST", &changesets, &key, Some(&body)),
        201,
        "create a change set",
    );
    let target = created["changeset"]["id"].as_str().expect("an id");
    let conflict = seed_conflict(client, &changesets, &key, &human, target, "owner");

    let listed = must(
        call(
            client,
            "GET",
            &format!("{changesets}/{target}/conflicts"),
            &key,
            None,
        ),
        200,
        "list the conflicts",
    );
    assert!(
        listed["conflicts"]
            .as_array()
            .is_some_and(|all| all.iter().any(|c| c["id"] == conflict.as_str())),
        "{listed}"
    );
    let resolve = case_named(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/conflicts/{conflict_id}/resolve",
    );
    let resolved = must(
        call(
            client,
            "POST",
            &format!(
                "/api/v1/w/{}/projects/{}/conflicts/{conflict}/resolve",
                f.a.id, f.a.project_id
            ),
            &key,
            resolve.body,
        ),
        200,
        "resolve the conflict",
    );
    assert_eq!(resolved["conflict"]["id"], conflict.as_str(), "{resolved}");
}

// ---------------------------------------------------------------------------
// Design tokens (US-028)
// ---------------------------------------------------------------------------

#[test]
fn the_token_routes_are_in_the_registry_and_the_suite_attacks_them() {
    for (method, path, access) in [
        (
            "PUT",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/tokens",
            Access::Agent,
        ),
        (
            "GET",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/tokens",
            Access::Both,
        ),
        (
            "GET",
            "/api/v1/w/{workspace_id}/projects/{project_id}/tokens",
            Access::Both,
        ),
    ] {
        let entry = registry()
            .into_iter()
            .find(|r| r.method == method && r.path == path)
            .unwrap_or_else(|| panic!("the registry has no route {method} {path}"));
        assert_eq!(entry.access, access, "{method} {path}");
        assert!(
            matches!(case_for(&entry).kind, Kind::Workspace),
            "{path} is a workspace route that the suite attacks"
        );
    }
}

#[test]
fn the_token_calls_of_the_attacks_work_for_the_workspace_that_owns_the_records() {
    // The control for the token routes: the owner's agent starts a design
    // system in a project of its own, imports the tokens of the case's body
    // and reads them back, and the human reads them too.
    let (f, client) = ready();
    let _guard = serial();
    let key = Auth::Key(&f.a.key);
    let human = Auth::Session(&f.a.token);
    let body = json!({ "name": unique("owner-tokens") }).to_string();
    let project = must(
        call(
            client,
            "POST",
            &format!("/api/v1/w/{}/projects", f.a.id),
            &key,
            Some(&body),
        ),
        201,
        "create a project",
    );
    let project = project["project"]["id"].as_str().expect("an id");
    let base = format!("/api/v1/w/{}/projects/{project}", f.a.id);
    let body = json!({ "title": unique("owner-tokens") }).to_string();
    let created = must(
        call(
            client,
            "POST",
            &format!("{base}/changesets"),
            &key,
            Some(&body),
        ),
        201,
        "create a change set",
    );
    let changeset = created["changeset"]["id"].as_str().expect("an id");
    let body = json!({ "kind": "design-system", "pillar": "ui", "title": unique("owner-tokens") })
        .to_string();
    must(
        call(
            client,
            "POST",
            &format!("{base}/changesets/{changeset}/docs"),
            &key,
            Some(&body),
        ),
        201,
        "start the design system doc",
    );
    let import = case_named(
        "PUT",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/tokens",
    );
    let tokens = format!("{base}/changesets/{changeset}/tokens");
    let imported = must(
        call(client, "PUT", &tokens, &key, import.body),
        200,
        "import the tokens",
    );
    assert_eq!(imported["modes"], json!(["light"]), "{imported}");
    for auth in [&key, &human] {
        let read = must(
            call(client, "GET", &tokens, auth, None),
            200,
            "read the draft's tokens",
        );
        assert_eq!(
            read["tokens"]["color"]["intruder"]["$value"], "#000000",
            "{read}"
        );
        must(
            call(client, "GET", &format!("{base}/tokens"), auth, None),
            200,
            "read the mainline tokens",
        );
    }
}

// ---------------------------------------------------------------------------
// Live design system writes (US-104)
// ---------------------------------------------------------------------------

/// The routes that write the design system's head without a change set.
const LIVE_DESIGN_SYSTEM_ROUTES: [&str; 2] = [
    "/api/v1/w/{workspace_id}/projects/{project_id}/tokens",
    "/api/v1/w/{workspace_id}/projects/{project_id}/components",
];

#[test]
fn the_live_design_system_routes_are_in_the_registry_and_the_suite_attacks_them() {
    for path in LIVE_DESIGN_SYSTEM_ROUTES {
        let entry = registry()
            .into_iter()
            .find(|r| r.method == "PUT" && r.path == path)
            .unwrap_or_else(|| panic!("the registry has no route PUT {path}"));
        assert_eq!(entry.access, Access::Agent, "PUT {path}");
        assert!(
            matches!(case_for(&entry).kind, Kind::Workspace),
            "PUT {path} is a workspace route that the suite attacks"
        );
    }
}

#[test]
fn the_live_design_system_calls_of_the_attacks_work_for_the_workspace_that_owns_the_records() {
    // The control for the live design system routes: the owner's agent starts
    // a design system without a change set in a project of its own, writes the
    // tokens and the components of the cases' bodies to its head, and both the
    // agent and the human read the tokens back.
    let (f, client) = ready();
    let _guard = serial();
    let key = Auth::Key(&f.a.key);
    let human = Auth::Session(&f.a.token);
    let body = json!({ "name": unique("owner-live-tokens") }).to_string();
    let project = must(
        call(
            client,
            "POST",
            &format!("/api/v1/w/{}/projects", f.a.id),
            &key,
            Some(&body),
        ),
        201,
        "create a project",
    );
    let project = project["project"]["id"].as_str().expect("an id");
    let base = format!("/api/v1/w/{}/projects/{project}", f.a.id);
    let body =
        json!({ "kind": "design-system", "pillar": "ui", "title": unique("owner-live-tokens") })
            .to_string();
    must(
        call(client, "POST", &format!("{base}/docs"), &key, Some(&body)),
        201,
        "start the design system doc without a change set",
    );
    let tokens = case_named("PUT", LIVE_DESIGN_SYSTEM_ROUTES[0]);
    let imported = must(
        call(client, "PUT", &format!("{base}/tokens"), &key, tokens.body),
        200,
        "write the tokens to the design system's head",
    );
    assert_eq!(imported["modes"], json!(["light"]), "{imported}");
    let components = case_named("PUT", LIVE_DESIGN_SYSTEM_ROUTES[1]);
    let imported = must(
        call(
            client,
            "PUT",
            &format!("{base}/components"),
            &key,
            components.body,
        ),
        200,
        "write the components to the design system's head",
    );
    assert_eq!(imported["components"][0]["name"], "Intruder", "{imported}");
    for auth in [&key, &human] {
        let read = must(
            call(client, "GET", &format!("{base}/tokens"), auth, None),
            200,
            "read the design system's tokens",
        );
        assert_eq!(
            read["tokens"]["color"]["intruder"]["$value"], "#000000",
            "{read}"
        );
    }
}

// ---------------------------------------------------------------------------
// Components and instances (US-029)
// ---------------------------------------------------------------------------

#[test]
fn the_component_routes_are_in_the_registry_and_the_suite_attacks_them() {
    for (method, path, access) in [
        (
            "PUT",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/components",
            Access::Agent,
        ),
        (
            "PUT",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/design",
            Access::Agent,
        ),
        (
            "GET",
            "/api/v1/w/{workspace_id}/projects/{project_id}/components/usage",
            Access::Both,
        ),
    ] {
        let entry = registry()
            .into_iter()
            .find(|r| r.method == method && r.path == path)
            .unwrap_or_else(|| panic!("the registry has no route {method} {path}"));
        assert_eq!(entry.access, access, "{method} {path}");
        assert!(
            matches!(case_for(&entry).kind, Kind::Workspace),
            "{path} is a workspace route that the suite attacks"
        );
    }
}

#[test]
fn the_component_calls_of_the_attacks_work_for_the_workspace_that_owns_the_records() {
    // The control for the component routes: the owner's agent starts a
    // design system and a design doc in a project of its own, imports the
    // components and the design of the cases' bodies, and the agent and the
    // human list the boards that use components.
    let (f, client) = ready();
    let _guard = serial();
    let key = Auth::Key(&f.a.key);
    let human = Auth::Session(&f.a.token);
    let body = json!({ "name": unique("owner-components") }).to_string();
    let project = must(
        call(
            client,
            "POST",
            &format!("/api/v1/w/{}/projects", f.a.id),
            &key,
            Some(&body),
        ),
        201,
        "create a project",
    );
    let project = project["project"]["id"].as_str().expect("an id");
    let base = format!("/api/v1/w/{}/projects/{project}", f.a.id);
    let body = json!({ "title": unique("owner-components") }).to_string();
    let created = must(
        call(
            client,
            "POST",
            &format!("{base}/changesets"),
            &key,
            Some(&body),
        ),
        201,
        "create a change set",
    );
    let changeset = created["changeset"]["id"].as_str().expect("an id");
    let mut docs = Vec::new();
    for (kind, title) in [("design-system", "owner-ds"), ("design", "owner-design")] {
        let body = json!({ "kind": kind, "pillar": "ui", "title": unique(title) }).to_string();
        let created = must(
            call(
                client,
                "POST",
                &format!("{base}/changesets/{changeset}/docs"),
                &key,
                Some(&body),
            ),
            201,
            "start a doc",
        );
        docs.push(created["doc"]["id"].as_str().expect("an id").to_string());
    }
    let design_doc = &docs[1];
    let import = case_named(
        "PUT",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/components",
    );
    let imported = must(
        call(
            client,
            "PUT",
            &format!("{base}/changesets/{changeset}/components"),
            &key,
            import.body,
        ),
        200,
        "import the components",
    );
    assert_eq!(imported["components"][0]["name"], "Intruder", "{imported}");
    let design = case_named(
        "PUT",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/design",
    );
    let placed = must(
        call(
            client,
            "PUT",
            &format!("{base}/changesets/{changeset}/docs/{design_doc}/design"),
            &key,
            design.body,
        ),
        200,
        "import the design",
    );
    assert_eq!(placed["pages"][0]["name"], "Intruder", "{placed}");
    for auth in [&key, &human] {
        let usage = must(
            call(
                client,
                "GET",
                &format!("{base}/components/usage"),
                auth,
                None,
            ),
            200,
            "list the boards that use components",
        );
        assert_eq!(usage["project_id"], project, "{usage}");
    }
}

// ---------------------------------------------------------------------------
// Pages and boards (US-030)
// ---------------------------------------------------------------------------

const DRAFT_BOARD_ROUTE: &str = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/boards/{board_id}";
const BOARD_ROUTE: &str =
    "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/boards/{board_id}";

#[test]
fn the_board_routes_are_in_the_registry_and_the_suite_attacks_them() {
    for path in [DRAFT_BOARD_ROUTE, BOARD_ROUTE] {
        let entry = registry()
            .into_iter()
            .find(|r| r.method == "GET" && r.path == path)
            .unwrap_or_else(|| panic!("the registry has no route GET {path}"));
        assert_eq!(entry.access, Access::Both, "GET {path}");
        assert!(
            matches!(case_for(&entry).kind, Kind::Workspace),
            "{path} is a workspace route that the suite attacks"
        );
    }
}

#[test]
fn the_board_reads_of_a_workspace_show_its_own_board_and_nothing_of_the_other() {
    // The control for the board routes, which the generic one skips: the
    // owner's agent imports a design into a doc of its own, and the agent and
    // the human read its board from the draft and, once it is saved and
    // merged, from the mainline and the version. The other workspace's credentials on the same
    // paths, and the owner's workspace with the other's IDs, are 404.
    let (f, client) = ready();
    let _guard = serial();
    for (own, other) in [(&f.a, &f.b), (&f.b, &f.a)] {
        let key = Auth::Key(&own.key);
        let human = Auth::Session(&own.token);
        let body = json!({ "name": unique("owner-boards") }).to_string();
        let project = must(
            call(
                client,
                "POST",
                &format!("/api/v1/w/{}/projects", own.id),
                &key,
                Some(&body),
            ),
            201,
            "create a project",
        );
        let project = project["project"]["id"].as_str().expect("an id");
        let base = format!("/api/v1/w/{}/projects/{project}", own.id);
        let body = json!({ "title": unique("owner-boards") }).to_string();
        let created = must(
            call(
                client,
                "POST",
                &format!("{base}/changesets"),
                &key,
                Some(&body),
            ),
            201,
            "create a change set",
        );
        let changeset = created["changeset"]["id"].as_str().expect("an id");
        let body = json!({ "kind": "design", "pillar": "ui", "title": unique("owner-boards") })
            .to_string();
        let created = must(
            call(
                client,
                "POST",
                &format!("{base}/changesets/{changeset}/docs"),
                &key,
                Some(&body),
            ),
            201,
            "start a design doc",
        );
        let doc = created["doc"]["id"].as_str().expect("an id");
        let text = unique("owner-board-text");
        let tags = format!(
            "<Page name=\"Owner page\">\n  <Board name=\"Owner board\" x={{0}} y={{0}} width={{320}} height={{200}}>\n    <Text name=\"Owner text\" content=\"{text}\" fontFamily=\"Inter\" fontSize={{12}}/>\n  </Board>\n</Page>"
        );
        let placed = must(
            call(
                client,
                "PUT",
                &format!("{base}/changesets/{changeset}/docs/{doc}/design"),
                &key,
                Some(&json!({ "tags": tags }).to_string()),
            ),
            200,
            "import the design",
        );
        let board = placed["pages"][0]["children"][0]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("the import answers the board's ID: {placed}"))
            .to_string();
        let fill_board = |template: &str, workspace: &str| {
            template
                .replace("{workspace_id}", workspace)
                .replace("{project_id}", project)
                .replace("{changeset_id}", changeset)
                .replace("{doc_id}", doc)
                .replace("{board_id}", &board)
        };
        let check = |path: &str| {
            for auth in [&key, &human] {
                let what = format!("{} with {}", route_name("GET", path), auth.label());
                let response = call(client, "GET", path, auth, None);
                assert_eq!(response.status, 200, "{what}: {}", response.text());
                let answer = response.json();
                assert_eq!(answer["board_id"], board.as_str(), "{what}: {answer}");
                assert_eq!(answer["doc_id"], doc, "{what}: {answer}");
                let tags = answer["tags"].as_str().unwrap_or_default();
                assert!(
                    tags.contains(&text),
                    "{what}: the answer holds the board's own content: {answer}"
                );
                for marker in other.markers() {
                    assert!(
                        !response.text().contains(marker),
                        "{what} shows {marker} of the other workspace: {}",
                        response.text()
                    );
                }
            }
            // The other workspace's credentials on the owner's real board.
            for auth in [Auth::Key(&other.key), Auth::Session(&other.token)] {
                let what = format!(
                    "{} with {} of the other workspace",
                    route_name("GET", path),
                    auth.label()
                );
                let response = call(client, "GET", path, &auth, None);
                assert_not_found_without_data(&response, own, &what);
                assert!(
                    !response.text().contains(&text),
                    "{what}: {}",
                    response.text()
                );
            }
        };
        check(&fill_board(DRAFT_BOARD_ROUTE, &own.id));

        let saved = must(
            call(
                client,
                "POST",
                &format!("{base}/changesets/{changeset}/docs/{doc}/versions"),
                &key,
                Some("{}"),
            ),
            201,
            "save a version",
        );
        let version = saved["version"]["id"].as_str().expect("an id").to_string();
        must(
            call(
                client,
                "POST",
                &format!("{base}/changesets/{changeset}/submit"),
                &key,
                None,
            ),
            200,
            "submit the change set",
        );
        let approved = must(
            call(
                client,
                "POST",
                &format!("{base}/changesets/{changeset}/approve"),
                &human,
                Some("{}"),
            ),
            200,
            "the human approves",
        );
        assert_eq!(approved["changeset"]["status"], "merged", "{approved}");
        check(&fill_board(BOARD_ROUTE, &own.id));
        check(&format!(
            "{}?version={version}",
            fill_board(BOARD_ROUTE, &own.id)
        ));

        // The owner's key, with the owner's IDs under the other workspace.
        for path in [
            fill_board(DRAFT_BOARD_ROUTE, &other.id),
            fill_board(BOARD_ROUTE, &other.id),
        ] {
            let response = call(client, "GET", &path, &key, None);
            let what = format!("{} with the owner's key", route_name("GET", &path));
            assert_not_found_without_data(&response, own, &what);
            assert!(
                !response.text().contains(&text),
                "{what}: {}",
                response.text()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Threads and phases (US-033)
// ---------------------------------------------------------------------------

const THREAD_ROUTES: [(&str, &str); 4] = [
    (
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/threads",
    ),
    (
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}",
    ),
    (
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/phases",
    ),
    (
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/posts",
    ),
];

#[test]
fn the_thread_routes_are_in_the_registry_and_the_suite_attacks_them() {
    for (method, path) in THREAD_ROUTES {
        let entry = registry()
            .into_iter()
            .find(|r| r.method == method && r.path == path)
            .unwrap_or_else(|| panic!("the registry has no route {method} {path}"));
        assert!(
            matches!(case_for(&entry).kind, Kind::Workspace),
            "{method} {path} is a workspace route that the suite attacks"
        );
    }

    let (f, _) = ready();
    for side in [&f.a, &f.b] {
        assert!(
            !side.thread_id.is_empty(),
            "the suite seeded no thread for {}",
            side.id
        );
    }
}

#[test]
fn the_thread_calls_of_the_attacks_work_for_the_workspace_that_owns_the_records() {
    // The control for the thread routes: the owner's agent starts a thread
    // with the body of the case, starts its next phase and posts in it, and
    // both the agent and the human read it, all in the owner's workspace.
    let (f, client) = ready();
    let _guard = serial();
    let key = Auth::Key(&f.a.key);
    let human = Auth::Session(&f.a.token);
    let path = |template: &str| fill(template, &f.a, &f.a);

    let (method, template) = THREAD_ROUTES[0];
    let created = must(
        call(
            client,
            method,
            &path(template),
            &key,
            case_named(method, template).body,
        ),
        201,
        "start a thread",
    );
    let thread = created["thread"]["id"].as_str().expect("an id");
    let threads = format!("/api/v1/w/{}/projects/{}/threads", f.a.id, f.a.project_id);

    let (method, template) = THREAD_ROUTES[2];
    must(
        call(
            client,
            method,
            &format!("{threads}/{thread}/phases"),
            &key,
            case_named(method, template).body,
        ),
        201,
        "start the next phase",
    );
    let (method, template) = THREAD_ROUTES[3];
    for auth in [&key, &human] {
        must(
            call(
                client,
                method,
                &format!("{threads}/{thread}/posts"),
                auth,
                case_named(method, template).body,
            ),
            201,
            "post in the thread",
        );
    }
    for auth in [&key, &human] {
        let shown = must(
            call(client, "GET", &format!("{threads}/{thread}"), auth, None),
            200,
            "show the thread",
        );
        assert_eq!(shown["thread"]["id"], thread, "{shown}");
    }
    for auth in [&key, &human] {
        let seeded = must(
            call(client, "GET", &path(THREAD_ROUTES[1].1), auth, None),
            200,
            "show the seeded thread",
        );
        assert_eq!(
            seeded["thread"]["question"], f.a.thread_question,
            "{seeded}"
        );
    }
}

// ---------------------------------------------------------------------------
// Directives (US-034)
// ---------------------------------------------------------------------------

const DIRECTIVES_ROUTE: &str =
    "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/directives";

#[test]
fn the_directive_route_is_a_human_route_that_the_suite_attacks() {
    let entry = registry()
        .into_iter()
        .find(|r| r.method == "POST" && r.path == DIRECTIVES_ROUTE)
        .unwrap_or_else(|| panic!("the registry has no route POST {DIRECTIVES_ROUTE}"));
    assert_eq!(entry.access, Access::Human, "POST {DIRECTIVES_ROUTE}");
    assert!(
        matches!(case_for(&entry).kind, Kind::Workspace),
        "POST {DIRECTIVES_ROUTE} is a workspace route that the suite attacks"
    );
}

#[test]
fn the_directive_call_of_the_attacks_works_for_the_workspace_that_owns_the_records() {
    // The control for the directive route: the owner's human posts a
    // directive with the body of the case in the seeded thread, and the
    // owner's agent is given it with its next request in the change set.
    let (f, client) = ready();
    let _guard = serial();
    let key = Auth::Key(&f.a.key);
    let human = Auth::Session(&f.a.token);
    let posted = must(
        call(
            client,
            "POST",
            &fill(DIRECTIVES_ROUTE, &f.a, &f.a),
            &human,
            case_named("POST", DIRECTIVES_ROUTE).body,
        ),
        201,
        "the human posts a directive",
    );
    assert_eq!(posted["post"]["type"], "directive", "{posted}");
    let shown = must(
        call(
            client,
            "GET",
            &fill(
                "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}",
                &f.a,
                &f.a,
            ),
            &key,
            None,
        ),
        200,
        "show the change set to the agent",
    );
    let given: Vec<&Value> = shown["directives"]
        .as_array()
        .map(|d| d.iter().collect())
        .unwrap_or_default();
    assert_eq!(given.len(), 1, "the agent is given the directive: {shown}");
    assert_eq!(given[0]["id"], posted["post"]["id"], "{shown}");
}

// ---------------------------------------------------------------------------
// Reading posts (US-035)
// ---------------------------------------------------------------------------

const POST_READ_ROUTES: [(&str, &str); 2] = [
    (
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/digest",
    ),
    (
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/posts/{post_id}",
    ),
];

#[test]
fn the_post_read_routes_are_in_the_registry_and_the_suite_attacks_them() {
    for (method, path) in POST_READ_ROUTES {
        let entry = registry()
            .into_iter()
            .find(|r| r.method == method && r.path == path)
            .unwrap_or_else(|| panic!("the registry has no route {method} {path}"));
        assert_eq!(entry.access, Access::Both, "{method} {path}");
        assert!(
            matches!(case_for(&entry).kind, Kind::Workspace),
            "{method} {path} is a workspace route that the suite attacks"
        );
    }

    let (f, _) = ready();
    for side in [&f.a, &f.b] {
        assert!(
            !side.post_id.is_empty(),
            "the suite seeded no post for {}",
            side.id
        );
    }
}

#[test]
fn the_post_read_calls_of_the_attacks_work_for_the_workspace_that_owns_the_records() {
    // The control for the post reads: the owner's agent, which wrote the
    // seeded post, and the owner's human read the thread's digest and the
    // post itself in the owner's workspace.
    let (f, client) = ready();
    let _guard = serial();
    let key = Auth::Key(&f.a.key);
    let human = Auth::Session(&f.a.token);
    let (digest, single) = (POST_READ_ROUTES[0].1, POST_READ_ROUTES[1].1);
    for auth in [&key, &human] {
        let listed = must(
            call(client, "GET", &fill(digest, &f.a, &f.a), auth, None),
            200,
            "read the seeded thread's digest",
        );
        let posts = listed["posts"]
            .as_array()
            .unwrap_or_else(|| panic!("the digest lists posts: {listed}"));
        assert!(
            posts
                .iter()
                .any(|p| p["id"] == f.a.post_id.as_str()
                    && p["summary"] == f.a.post_summary.as_str()),
            "the digest holds the seeded post: {listed}"
        );
        let read = must(
            call(client, "GET", &fill(single, &f.a, &f.a), auth, None),
            200,
            "read the seeded post",
        );
        assert_eq!(read["post"]["id"], f.a.post_id.as_str(), "{read}");
        assert_eq!(read["post"]["summary"], f.a.post_summary.as_str(), "{read}");
    }
}

// ---------------------------------------------------------------------------
// Decisions (US-037)
// ---------------------------------------------------------------------------

const DECISION_ROUTES: [(&str, &str, Access); 5] = [
    (
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/decisions",
        Access::Agent,
    ),
    (
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/decisions/{decision_id}",
        Access::Both,
    ),
    (
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/decisions/{decision_id}/approve",
        Access::Human,
    ),
    (
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/decisions/{decision_id}/reject",
        Access::Human,
    ),
    (
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/decisions/{decision_id}",
        Access::Agent,
    ),
];

#[test]
fn the_decision_routes_are_in_the_registry_and_the_suite_attacks_them() {
    for (method, path, access) in DECISION_ROUTES {
        let entry = registry()
            .into_iter()
            .find(|r| r.method == method && r.path == path)
            .unwrap_or_else(|| panic!("the registry has no route {method} {path}"));
        assert_eq!(entry.access, access, "{method} {path}");
        assert!(
            matches!(case_for(&entry).kind, Kind::Workspace),
            "{method} {path} is a workspace route that the suite attacks"
        );
    }

    let (f, _) = ready();
    for side in [&f.a, &f.b] {
        assert!(
            !side.decision_id.is_empty(),
            "the suite seeded no decision for {}",
            side.id
        );
    }
}

#[test]
fn the_decision_calls_of_the_attacks_work_for_the_workspace_that_owns_the_records() {
    // The control for the decision routes: in the owner's workspace, the
    // owner's agent proposes a decision with the body of the case for a new
    // thread of the seeded change set and links the change set to it, both
    // the agent and the human read it and the seeded decision, and the human
    // approves it with the body of the case; a second proposal is rejected
    // with the body of the case.
    let (f, client) = ready();
    let _guard = serial();
    let key = Auth::Key(&f.a.key);
    let human = Auth::Session(&f.a.token);
    let path = |template: &str| fill(template, &f.a, &f.a);
    let body = |method: &str, template: &str| case_named(method, template).body;

    let (method, template) = THREAD_ROUTES[0];
    let created = must(
        call(
            client,
            method,
            &path(template),
            &key,
            body(method, template),
        ),
        201,
        "start a thread",
    );
    let thread = created["thread"]["id"].as_str().expect("an id");
    let project = format!("/api/v1/w/{}/projects/{}", f.a.id, f.a.project_id);
    let (propose, _, _) = DECISION_ROUTES[0];
    let propose_body = body(propose, DECISION_ROUTES[0].1);
    let proposed = must(
        call(
            client,
            propose,
            &format!("{project}/threads/{thread}/decisions"),
            &key,
            propose_body,
        ),
        201,
        "propose a decision",
    );
    let decision = proposed["decision"]["id"].as_str().expect("an id");
    for auth in [&key, &human] {
        let read = must(
            call(
                client,
                "GET",
                &format!("{project}/decisions/{decision}"),
                auth,
                None,
            ),
            200,
            "read the decision",
        );
        assert_eq!(read["decision"]["id"], decision, "{read}");
        let seeded = must(
            call(client, "GET", &path(DECISION_ROUTES[1].1), auth, None),
            200,
            "read the seeded decision",
        );
        assert_eq!(
            seeded["decision"]["direction"],
            f.a.decision_direction.as_str(),
            "{seeded}"
        );
    }
    let (method, template, _) = DECISION_ROUTES[4];
    let linked = must(
        call(
            client,
            method,
            &format!(
                "{project}/changesets/{}/decisions/{decision}",
                f.a.changeset_id
            ),
            &key,
            body(method, template),
        ),
        200,
        "link the change set to the decision",
    );
    assert!(
        linked["changeset"]["decisions"]
            .as_array()
            .is_some_and(|list| list.iter().any(|d| d["id"] == decision)),
        "{linked}"
    );
    let (method, template, _) = DECISION_ROUTES[2];
    let approved = must(
        call(
            client,
            method,
            &format!("{project}/decisions/{decision}/approve"),
            &human,
            body(method, template),
        ),
        200,
        "approve the decision",
    );
    assert_eq!(approved["decision"]["status"], "approved", "{approved}");

    let proposed = must(
        call(
            client,
            propose,
            &format!("{project}/threads/{thread}/decisions"),
            &key,
            propose_body,
        ),
        201,
        "propose a second decision",
    );
    let second = proposed["decision"]["id"].as_str().expect("an id");
    let (method, template, _) = DECISION_ROUTES[3];
    let rejected = must(
        call(
            client,
            method,
            &format!("{project}/decisions/{second}/reject"),
            &human,
            body(method, template),
        ),
        200,
        "reject the decision",
    );
    assert_eq!(rejected["decision"]["status"], "rejected", "{rejected}");
}

// ---------------------------------------------------------------------------
// Comments (US-038)
// ---------------------------------------------------------------------------

const COMMENT_ROUTES: [(&str, &str, Access); 6] = [
    (
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/versions/{version_id}/comments",
        Access::Both,
    ),
    (
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/comments",
        Access::Both,
    ),
    (
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/comments/{comment_id}",
        Access::Both,
    ),
    (
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/comments/{comment_id}/replies",
        Access::Both,
    ),
    // US-106: comments on the live head, and the human's delete.
    (
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/comments",
        Access::Both,
    ),
    (
        "DELETE",
        "/api/v1/w/{workspace_id}/projects/{project_id}/comments/{comment_id}",
        Access::Human,
    ),
];

#[test]
fn the_comment_calls_of_the_attacks_work_for_the_workspace_that_owns_the_records() {
    // The control for the comment routes: in the owner's workspace, the
    // owner's human comments on the seeded version with the body of the case,
    // both the agent and the human reply with the body of the case and read
    // the comment, the seeded comment and the list; the agent comments on a
    // paragraph of a live doc's head with the body of the case (its node put
    // in for `<comment_node>`), and the human deletes that comment with the
    // call of the case.
    let (f, client) = ready();
    let _guard = serial();
    let key = Auth::Key(&f.a.key);
    let human = Auth::Session(&f.a.token);
    let path = |template: &str| fill(template, &f.a, &f.a);
    let body = |index: usize| {
        let (method, template, _) = COMMENT_ROUTES[index];
        body_of(case_named(method, template), &f.a)
    };
    let project = format!("/api/v1/w/{}/projects/{}", f.a.id, f.a.project_id);

    let made = must(
        call(
            client,
            "POST",
            &path(COMMENT_ROUTES[0].1),
            &human,
            body(0).as_deref(),
        ),
        201,
        "comment on the seeded version",
    );
    let comment = made["comment"]["id"].as_str().expect("an id");
    for auth in [&key, &human] {
        must(
            call(
                client,
                "POST",
                &format!("{project}/comments/{comment}/replies"),
                auth,
                body(3).as_deref(),
            ),
            201,
            "reply to the comment",
        );
        let read = must(
            call(
                client,
                "GET",
                &format!("{project}/comments/{comment}"),
                auth,
                None,
            ),
            200,
            "read the comment",
        );
        assert_eq!(read["comment"]["id"], comment, "{read}");
        let seeded = must(
            call(client, "GET", &path(COMMENT_ROUTES[2].1), auth, None),
            200,
            "read the seeded comment",
        );
        assert_eq!(
            seeded["comment"]["text"],
            f.a.comment_text.as_str(),
            "{seeded}"
        );
        let listed = must(
            call(client, "GET", &path(COMMENT_ROUTES[1].1), auth, None),
            200,
            "list the comments",
        );
        assert!(
            listed["comments"].as_array().is_some_and(|list| {
                list.iter().any(|c| c["id"] == comment)
                    && list.iter().any(|c| c["id"] == f.a.comment_id.as_str())
            }),
            "{listed}"
        );
    }

    // A live doc of the owner's with one paragraph in its head.
    let created = must(
        call(
            client,
            "POST",
            &format!("{project}/docs"),
            &key,
            Some(
                &json!({ "kind": "html", "pillar": "spec", "title": unique("owner-live-doc") })
                    .to_string(),
            ),
        ),
        201,
        "create a doc without a change set",
    );
    let live = created["doc"]["id"].as_str().expect("an id").to_string();
    let edited = must(
        call(
            client,
            "POST",
            &format!("{project}/docs/{live}/ops"),
            &key,
            Some(
                &json!({ "ops": [{
                    "op": "insert",
                    "parent": null,
                    "position": { "after": null },
                    "content": "<p>The owner's paragraph.</p>"
                }] })
                .to_string(),
            ),
        ),
        200,
        "write a paragraph to the live doc's head",
    );
    let node = edited["results"][0]["node"].as_str().expect("a node");
    let (_, template, _) = COMMENT_ROUTES[4];
    let live_body = case_named("POST", template)
        .body
        .map(|b| b.replace("<comment_node>", node));
    let pinned = must(
        call(
            client,
            "POST",
            &format!("{project}/docs/{live}/comments"),
            &key,
            live_body.as_deref(),
        ),
        201,
        "comment on the live doc's head",
    );
    let pinned = pinned["comment"]["id"].as_str().expect("an id");
    let response = call(
        client,
        "DELETE",
        &format!("{project}/comments/{pinned}"),
        &human,
        None,
    );
    assert_eq!(
        response.status,
        204,
        "delete the comment: {}",
        response.text()
    );
    must(
        call(
            client,
            "GET",
            &format!("{project}/comments/{comment}"),
            &human,
            None,
        ),
        200,
        "the other comment is still there",
    );
}

// ---------------------------------------------------------------------------
// The project home (US-040)
// ---------------------------------------------------------------------------

const HOME_ROUTE: &str = "/api/v1/w/{workspace_id}/projects/{project_id}/home";

#[test]
fn the_home_route_is_a_human_route_that_the_suite_attacks() {
    let entry = registry()
        .into_iter()
        .find(|r| r.method == "GET" && r.path == HOME_ROUTE)
        .unwrap_or_else(|| panic!("the registry has no route GET {HOME_ROUTE}"));
    assert_eq!(entry.access, Access::Human, "GET {HOME_ROUTE}");
    assert!(
        matches!(case_for(&entry).kind, Kind::Workspace),
        "GET {HOME_ROUTE} is a workspace route that the suite attacks"
    );
}

#[test]
fn the_home_call_of_the_attacks_shows_the_owner_its_own_records() {
    // The control for the home route: the owner's human reads the home of the
    // seeded project, which lists the seeded doc, change set and thread, and
    // nothing of the other workspace.
    let (f, client) = ready();
    let _guard = serial();
    let human = Auth::Session(&f.a.token);
    let home = must(
        call(client, "GET", &fill(HOME_ROUTE, &f.a, &f.a), &human, None),
        200,
        "the human reads the project home",
    );
    assert_eq!(home["project_id"], f.a.project_id.as_str(), "{home}");
    let ids = |list: &str| -> Vec<String> {
        home[list]
            .as_array()
            .unwrap_or_else(|| panic!("the home lists `{list}`: {home}"))
            .iter()
            .filter_map(|item| item["id"].as_str().map(str::to_string))
            .collect()
    };
    assert!(ids("docs").contains(&f.a.doc_id), "{home}");
    assert!(ids("changesets").contains(&f.a.changeset_id), "{home}");
    assert!(ids("threads").contains(&f.a.thread_id), "{home}");
    for marker in f.b.markers() {
        assert!(
            !home.to_string().contains(marker),
            "the home shows {marker} of the other workspace: {home}"
        );
    }
}

// ---------------------------------------------------------------------------
// The change set review (US-046)
// ---------------------------------------------------------------------------

const REVIEW_ROUTE: &str =
    "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/review";

#[test]
fn the_review_route_is_a_human_route_that_the_suite_attacks() {
    let entry = registry()
        .into_iter()
        .find(|r| r.method == "GET" && r.path == REVIEW_ROUTE)
        .unwrap_or_else(|| panic!("the registry has no route GET {REVIEW_ROUTE}"));
    assert_eq!(entry.access, Access::Human, "GET {REVIEW_ROUTE}");
    assert!(
        matches!(case_for(&entry).kind, Kind::Workspace),
        "GET {REVIEW_ROUTE} is a workspace route that the suite attacks"
    );
}

// ---------------------------------------------------------------------------
// Features (US-059, US-110)
// ---------------------------------------------------------------------------

const FEATURE_ROUTES: [(&str, &str, Access); 5] = [
    (
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/features",
        Access::Agent,
    ),
    (
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/features",
        Access::Both,
    ),
    (
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}",
        Access::Both,
    ),
    (
        "PATCH",
        "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}",
        Access::Agent,
    ),
    (
        "DELETE",
        "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}",
        Access::Agent,
    ),
];

#[test]
fn the_feature_routes_are_in_the_registry_and_the_suite_attacks_them() {
    for (method, path, access) in FEATURE_ROUTES {
        let entry = registry()
            .into_iter()
            .find(|r| r.method == method && r.path == path)
            .unwrap_or_else(|| panic!("the registry has no route {method} {path}"));
        assert_eq!(entry.access, access, "{method} {path}");
        assert!(
            matches!(case_for(&entry).kind, Kind::Workspace),
            "{method} {path} is a workspace route that the suite attacks"
        );
    }

    let (f, _) = ready();
    for side in [&f.a, &f.b] {
        assert!(
            !side.feature_id.is_empty(),
            "the suite seeded no feature for {}",
            side.id
        );
    }
}

#[test]
fn the_feature_calls_of_the_attacks_work_for_the_workspace_that_owns_the_records() {
    // The control for the feature routes: in the owner's workspace, the
    // owner's agent creates a feature with the body of the case, and both the
    // agent and the human list the project's features, in creation order, and
    // read the new one and the seeded one. Then the agent renames the new
    // feature and deletes it with the calls of the cases, and the seeded one
    // is still there.
    let (f, client) = ready();
    let _guard = serial();
    let key = Auth::Key(&f.a.key);
    let human = Auth::Session(&f.a.token);
    let path = |template: &str| fill(template, &f.a, &f.a);

    let (method, template, _) = FEATURE_ROUTES[0];
    let body = case_named(method, template)
        .body
        .map(|b| b.replace("intruder", &unique("owner")));
    let created = must(
        call(client, method, &path(template), &key, body.as_deref()),
        201,
        "create a feature",
    );
    let feature = created["feature"]["id"].as_str().expect("an id");
    let project = format!("/api/v1/w/{}/projects/{}", f.a.id, f.a.project_id);
    for auth in [&key, &human] {
        let listed = must(
            call(client, "GET", &path(FEATURE_ROUTES[1].1), auth, None),
            200,
            "list the features",
        );
        let ids: Vec<&str> = listed["features"]
            .as_array()
            .unwrap_or_else(|| panic!("a list of features: {listed}"))
            .iter()
            .filter_map(|item| item["id"].as_str())
            .collect();
        let seeded = ids.iter().position(|id| *id == f.a.feature_id);
        let new = ids.iter().position(|id| *id == feature);
        assert!(
            seeded.is_some() && new.is_some() && seeded < new,
            "the list has the seeded feature before the new one: {listed}"
        );
        let read = must(
            call(
                client,
                "GET",
                &format!("{project}/features/{feature}"),
                auth,
                None,
            ),
            200,
            "read the feature",
        );
        assert_eq!(read["feature"]["id"], feature, "{read}");
        let seeded = must(
            call(client, "GET", &path(FEATURE_ROUTES[2].1), auth, None),
            200,
            "read the seeded feature",
        );
        assert_eq!(
            seeded["feature"]["name"],
            f.a.feature_name.as_str(),
            "{seeded}"
        );
    }

    let (method, template, _) = FEATURE_ROUTES[3];
    let name = unique("renamed");
    let body = case_named(method, template)
        .body
        .map(|b| b.replace("intruder", &name));
    let renamed = must(
        call(
            client,
            method,
            &format!("{project}/features/{feature}"),
            &key,
            body.as_deref(),
        ),
        200,
        "rename the feature",
    );
    assert_eq!(renamed["feature"]["id"], feature, "{renamed}");
    assert_eq!(renamed["feature"]["name"], name.as_str(), "{renamed}");
    let (method, _, _) = FEATURE_ROUTES[4];
    let response = call(
        client,
        method,
        &format!("{project}/features/{feature}"),
        &key,
        None,
    );
    assert_eq!(
        response.status,
        204,
        "delete the feature: {}",
        response.text()
    );
    let gone = call(
        client,
        "GET",
        &format!("{project}/features/{feature}"),
        &human,
        None,
    );
    assert_eq!(gone.status, 404, "the deleted feature: {}", gone.text());
    must(
        call(client, "GET", &path(FEATURE_ROUTES[2].1), &human, None),
        200,
        "the seeded feature is still there",
    );
}

// ---------------------------------------------------------------------------
// The doc list (US-061)
// ---------------------------------------------------------------------------

const DOCS_ROUTE: &str = "/api/v1/w/{workspace_id}/projects/{project_id}/docs";

#[test]
fn the_doc_list_route_is_in_the_registry_and_the_suite_attacks_it() {
    let entry = registry()
        .into_iter()
        .find(|r| r.method == "GET" && r.path == DOCS_ROUTE)
        .unwrap_or_else(|| panic!("the registry has no route GET {DOCS_ROUTE}"));
    assert_eq!(entry.access, Access::Both, "GET {DOCS_ROUTE}");
    assert!(
        matches!(case_for(&entry).kind, Kind::Workspace),
        "GET {DOCS_ROUTE} is a workspace route that the suite attacks"
    );
}

#[test]
fn the_doc_list_of_the_attacks_works_for_the_workspace_that_owns_the_records() {
    // The control for the doc list: in the owner's workspace, the agent and
    // the human list the project's docs, which include the seeded doc and
    // none of the other workspace's.
    let (f, client) = ready();
    let _guard = serial();
    let key = Auth::Key(&f.a.key);
    let human = Auth::Session(&f.a.token);
    for auth in [&key, &human] {
        let listed = must(
            call(client, "GET", &fill(DOCS_ROUTE, &f.a, &f.a), auth, None),
            200,
            "list the docs",
        );
        let ids: Vec<&str> = listed["docs"]
            .as_array()
            .unwrap_or_else(|| panic!("a list of docs: {listed}"))
            .iter()
            .filter_map(|item| item["id"].as_str())
            .collect();
        assert!(
            ids.contains(&f.a.doc_id.as_str()),
            "the list has the seeded doc: {listed}"
        );
        assert!(
            !ids.contains(&f.b.doc_id.as_str()),
            "the list has no doc of the other workspace: {listed}"
        );
    }
}

// ---------------------------------------------------------------------------
// Human authority
// ---------------------------------------------------------------------------

#[test]
fn every_human_route_answers_an_api_key_with_403_and_changes_nothing() {
    let (f, client) = ready();
    let _guard = serial();
    let before = (snapshot(&f.a.id), snapshot(&f.b.id));
    let nobody = Side::stranger(UNKNOWN_ID);
    let garbage = Side::stranger("not-a-uuid");
    let mut attacks = 0;
    let mut routes = 0;
    for entry in registry() {
        if entry.access != Access::Human {
            continue;
        }
        let exempt = KEY_EXEMPT
            .iter()
            .any(|(m, p)| *m == entry.method && *p == entry.path);
        if exempt {
            continue;
        }
        let case = case_for(&entry);
        routes += 1;
        // The paths: the key's own workspace and records, the other
        // workspace's, ones that do not exist, and ones that are no UUIDs. The
        // authority check comes first, so all of them are 403.
        let paths = [
            fill(&entry.path, &f.a, &f.a),
            fill(&entry.path, &f.b, &f.b),
            fill(&entry.path, &f.a, &f.b),
            fill(&entry.path, &nobody, &nobody),
            fill(&entry.path, &garbage, &garbage),
        ];
        for path in paths {
            let callers = [
                Auth::Key(&f.a.key),
                Auth::Key(&f.b.key),
                // Next to a valid cookie, the key still decides: this is an agent.
                Auth::KeyAndSession(&f.a.key, &f.a.token),
            ];
            for auth in callers {
                let response = call(client, &entry.method, &path, &auth, case.body);
                let what = format!("{} with {}", route_name(&entry.method, &path), auth.label());
                assert_eq!(response.status, 403, "{what}: {}", response.text());
                assert_eq!(response.json(), json!({ "error": "forbidden" }), "{what}");
                attacks += 1;
            }
        }
    }
    assert!(routes >= 5, "only {routes} human routes were attacked");
    assert!(attacks >= 60, "only {attacks} attacks ran");

    // Nothing happened: no row changed, the human's session lives on and the
    // key the attacks used was not revoked.
    assert_eq!(
        (snapshot(&f.a.id), snapshot(&f.b.id)),
        before,
        "a refused call changed rows"
    );
    let me = call(
        client,
        "GET",
        "/api/v1/auth/me",
        &Auth::Session(&f.a.token),
        None,
    );
    assert_eq!(me.status, 200, "alice's session survived: {}", me.text());
    let whoami = format!("/api/v1/w/{}/whoami", f.a.id);
    let response = call(client, "GET", &whoami, &Auth::Key(&f.a.key), None);
    assert_eq!(
        response.status,
        200,
        "A's key survived: {}",
        response.text()
    );
}

#[test]
fn the_login_is_the_only_human_route_that_an_api_key_does_not_turn_down() {
    // The exemption list is closed: a new human route that a key could use
    // would need a decision here.
    assert_eq!(KEY_EXEMPT, [("POST", "/api/v1/auth/login")]);
    let human_routes: Vec<String> = registry()
        .into_iter()
        .filter(|r| r.access == Access::Human)
        .map(|r| route_name(&r.method, &r.path))
        .collect();
    assert!(
        human_routes.contains(&"POST /api/v1/auth/login".to_string()),
        "{human_routes:?}"
    );
    let (f, client) = ready();
    let body = json!({ "email": f.a.email, "password": "not the password" }).to_string();
    let response = call(
        client,
        "POST",
        "/api/v1/auth/login",
        &Auth::Key(&f.a.key),
        Some(&body),
    );
    assert_ne!(
        response.status,
        403,
        "the login checks the password: {}",
        response.text()
    );
}

// ---------------------------------------------------------------------------
// WebSockets
// ---------------------------------------------------------------------------

/// The answer to a handshake that did not open the socket.
#[derive(Debug)]
struct Refused {
    status: u16,
    body: String,
}

struct Ws {
    stream: TcpStream,
    buffer: Vec<u8>,
}

enum Arrived {
    Data(Vec<u8>),
    Timeout,
    Eof,
}

impl Ws {
    fn open(client: &Client, path: &str, auth: &Auth) -> Result<Ws, Refused> {
        let stream = TcpStream::connect(("127.0.0.1", client.port())).expect("connect");
        let mut ws = Ws {
            stream,
            buffer: Vec::new(),
        };
        let mut head = format!(
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nUpgrade: websocket\r\n\
             Connection: Upgrade\r\nSec-WebSocket-Key: {HANDSHAKE_KEY}\r\n\
             Sec-WebSocket-Version: 13\r\n"
        );
        for (name, value) in auth.headers() {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("\r\n");
        ws.stream
            .write_all(head.as_bytes())
            .expect("send handshake");

        let deadline = Instant::now() + PATIENCE;
        let split = loop {
            if let Some(at) = ws.buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                break at;
            }
            match ws.read_some(deadline) {
                Arrived::Data(bytes) => ws.buffer.extend(bytes),
                _ => panic!(
                    "no complete handshake answer for {path}: {:?}",
                    String::from_utf8_lossy(&ws.buffer)
                ),
            }
        };
        let head = String::from_utf8_lossy(&ws.buffer[..split]).into_owned();
        let status: u16 = head
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|s| s.parse().ok())
            .expect("status line");
        ws.buffer.drain(..split + 4);
        if status == 101 {
            return Ok(ws);
        }
        let length: usize = head
            .lines()
            .filter_map(|l| l.split_once(':'))
            .find(|(n, _)| n.trim().eq_ignore_ascii_case("content-length"))
            .and_then(|(_, v)| v.trim().parse().ok())
            .unwrap_or(0);
        while ws.buffer.len() < length {
            match ws.read_some(deadline) {
                Arrived::Data(bytes) => ws.buffer.extend(bytes),
                _ => break,
            }
        }
        Err(Refused {
            status,
            body: String::from_utf8_lossy(&ws.buffer).into_owned(),
        })
    }

    /// Reads whatever arrives before `deadline`.
    fn read_some(&mut self, deadline: Instant) -> Arrived {
        let wait = deadline
            .saturating_duration_since(Instant::now())
            .max(Duration::from_millis(1));
        self.stream
            .set_read_timeout(Some(wait))
            .expect("set the read timeout");
        let mut chunk = [0u8; 8192];
        match self.stream.read(&mut chunk) {
            Ok(0) => Arrived::Eof,
            Ok(n) => Arrived::Data(chunk[..n].to_vec()),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                Arrived::Timeout
            }
            Err(_) => Arrived::Eof,
        }
    }

    /// One complete frame off the front of the buffer, as (opcode, payload).
    fn take_frame(&mut self) -> Option<(u8, Vec<u8>)> {
        let b = &self.buffer;
        if b.len() < 2 {
            return None;
        }
        let opcode = b[0] & 0x0f;
        let mut length = (b[1] & 0x7f) as usize;
        let mut at = 2;
        if length == 126 {
            if b.len() < 4 {
                return None;
            }
            length = u16::from_be_bytes([b[2], b[3]]) as usize;
            at = 4;
        } else if length == 127 {
            if b.len() < 10 {
                return None;
            }
            length = u64::from_be_bytes(b[2..10].try_into().unwrap()) as usize;
            at = 10;
        }
        // The server's frames are not masked.
        if b.len() < at + length {
            return None;
        }
        let payload = b[at..at + length].to_vec();
        self.buffer.drain(..at + length);
        Some((opcode, payload))
    }

    fn send(&mut self, message: &Value) {
        let payload = message.to_string().into_bytes();
        let mask = [0x12u8, 0x34, 0x56, 0x78];
        let mut frame = vec![0x81];
        match payload.len() {
            n if n < 126 => frame.push(0x80 | n as u8),
            n => {
                frame.push(0x80 | 126);
                frame.extend((n as u16).to_be_bytes());
            }
        }
        frame.extend(mask);
        frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        self.stream.write_all(&frame).expect("send a frame");
    }

    /// The next JSON message from the server, `None` after `wait` of silence or
    /// when the socket closes.
    fn next(&mut self, wait: Duration) -> Option<Value> {
        let deadline = Instant::now() + wait;
        loop {
            while let Some((opcode, payload)) = self.take_frame() {
                match opcode {
                    1 => {
                        let text = String::from_utf8_lossy(&payload).into_owned();
                        return Some(
                            serde_json::from_str(&text)
                                .unwrap_or_else(|e| panic!("a text frame is JSON ({e}): {text}")),
                        );
                    }
                    8 => return None,
                    _ => {}
                }
            }
            match self.read_some(deadline) {
                Arrived::Data(bytes) => self.buffer.extend(bytes),
                Arrived::Timeout if Instant::now() < deadline => {}
                Arrived::Timeout | Arrived::Eof => return None,
            }
        }
    }

    /// Every message that arrives until the socket has been quiet for `QUIET`.
    fn drain(&mut self) -> Vec<Value> {
        let mut found = Vec::new();
        while let Some(message) = self.next(QUIET) {
            found.push(message);
        }
        found
    }
}

fn assert_refused(result: Result<Ws, Refused>, status: u16, code: &str, what: &str) {
    let refused = match result {
        Ok(_) => panic!("{what}: the socket opened, but {status} {code} was expected"),
        Err(refused) => refused,
    };
    assert_eq!(refused.status, status, "{what}: {refused:?}");
    let body: Value = serde_json::from_str(&refused.body)
        .unwrap_or_else(|e| panic!("{what}: the refusal is JSON ({e}): {refused:?}"));
    assert_eq!(body, json!({ "error": code }), "{what}");
}

#[test]
fn the_channel_of_workspace_b_is_refused_to_the_credentials_of_workspace_a() {
    let (f, client) = ready();
    let routes = websocket_registry();
    assert!(!routes.is_empty(), "the router has WebSocket routes");
    for route in routes {
        let theirs = fill(&route.path, &f.b, &f.b);
        let own = fill(&route.path, &f.a, &f.a);
        for auth in [
            Auth::Key(&f.a.key),
            Auth::Session(&f.a.token),
            Auth::KeyAndSession(&f.a.key, &f.a.token),
        ] {
            let what = format!("{theirs} with {} of workspace A", auth.label());
            assert_refused(Ws::open(client, &theirs, &auth), 404, "not_found", &what);
            // The control: the same credentials open the channel of A.
            assert!(
                Ws::open(client, &own, &auth).is_ok(),
                "{own} with {} should open",
                auth.label()
            );
        }
        // The channel is the one in the path, whatever the query says.
        let sneaky = format!("{own}?workspace_id={}", f.b.id);
        assert!(Ws::open(client, &sneaky, &Auth::Key(&f.a.key)).is_ok());
        let sneaky = format!("{theirs}?workspace_id={}", f.a.id);
        assert_refused(
            Ws::open(client, &sneaky, &Auth::Key(&f.a.key)),
            404,
            "not_found",
            "B's channel with A named in the query",
        );
        assert_refused(
            Ws::open(client, &theirs, &Auth::Nobody),
            401,
            "unauthorized",
            "B's channel without credentials",
        );
    }
}

#[test]
fn a_socket_of_workspace_a_cannot_subscribe_to_a_project_of_workspace_b() {
    let (f, client) = ready();
    let _guard = serial();
    let channel = format!("/ws/w/{}", f.a.id);
    let mut ws = Ws::open(client, &channel, &Auth::Key(&f.a.key)).expect("A's channel opens");

    for after in [None, Some(0)] {
        let mut message = json!({ "type": "subscribe", "project_id": f.b.project_id });
        if let Some(after) = after {
            message["after"] = json!(after);
        }
        ws.send(&message);
        let reply = ws.next(PATIENCE).expect("the server answers");
        assert_eq!(
            reply,
            json!({ "type": "error", "error": "not_found" }),
            "subscribing to B's project (after {after:?})"
        );
    }

    // Workspace B goes on working, and none of it reaches the socket of A,
    // which also holds a subscription of its own to prove it is listening.
    ws.send(&json!({ "type": "subscribe", "project_id": f.a.project_id }));
    let ack = ws.next(PATIENCE).expect("the server answers");
    assert_eq!(ack["type"], "subscribed", "A's own project: {ack}");
    let created = call(
        client,
        "POST",
        &format!(
            "/api/v1/w/{}/projects/{}/changesets",
            f.b.id, f.b.project_id
        ),
        &Auth::Key(&f.b.key),
        Some(&json!({ "title": unique("b-only") }).to_string()),
    );
    must(created, 201, "B's agent creates a change set");
    let heard = ws.drain();
    assert!(
        heard.is_empty(),
        "the socket of workspace A heard of workspace B: {heard:?}"
    );

    // The control: a change in A is heard.
    let created = call(
        client,
        "POST",
        &format!(
            "/api/v1/w/{}/projects/{}/changesets",
            f.a.id, f.a.project_id
        ),
        &Auth::Key(&f.a.key),
        Some(&json!({ "title": unique("a-only") }).to_string()),
    );
    must(created, 201, "A's agent creates a change set");
    let event = ws.next(PATIENCE).expect("A hears its own event");
    assert_eq!(event["type"], "event", "{event}");
    assert_eq!(event["event"], "changeset.open", "{event}");
}

// ---------------------------------------------------------------------------
// Row-level security at the database
// ---------------------------------------------------------------------------

fn sqlstate(error: &sqlx::Error) -> String {
    error
        .as_database_error()
        .and_then(|e| e.code().map(|c| c.into_owned()))
        .unwrap_or_default()
}

/// Runs `sql` as the API role with `workspace` set, in a transaction that is
/// rolled back: the rows it changed, or the SQLSTATE of the error.
async fn attempt(conn: &mut PgConnection, workspace: &str, sql: &str) -> Result<u64, String> {
    let mut tx = conn.begin().await.expect("begin");
    sqlx::query("select set_config('app.workspace_id', $1, true)")
        .bind(workspace)
        .execute(&mut *tx)
        .await
        .expect("set app.workspace_id");
    let outcome = sqlx::query(sql)
        .execute(&mut *tx)
        .await
        .map(|done| done.rows_affected())
        .map_err(|e| format!("{} ({e})", sqlstate(&e)));
    drop(tx);
    outcome
}

/// The tenant tables, each with rows of both workspaces: a table whose rows the
/// suite did not seed would make every check below vacuous.
async fn seeded_tenant_tables(a: &str, b: &str) -> Vec<String> {
    let mut admin = admin_connection().await;
    let tables = tenant_tables(&mut admin).await;
    for table in &tables {
        for workspace in [a, b] {
            assert!(
                rows_of(&mut admin, table, workspace).await > 0,
                "the suite seeded no row of {table} for workspace {workspace}: \
                 make fixture() in tests/isolation.rs create one"
            );
        }
    }
    tables
}

#[test]
fn row_level_security_shows_workspace_a_none_of_the_rows_of_b_in_any_tenant_table() {
    let f = fixture();
    let _guard = serial();
    block(async {
        let tables = seeded_tenant_tables(&f.a.id, &f.b.id).await;
        assert!(tables.len() >= 7, "the tenant tables: {tables:?}");
        let mut admin = admin_connection().await;
        let mut conn = api_connection().await;
        for (own, other) in [(&f.a.id, &f.b.id), (&f.b.id, &f.a.id)] {
            for table in &tables {
                let column = scope_column(table);
                let mut tx = conn.begin().await.expect("begin");
                sqlx::query("select set_config('app.workspace_id', $1, true)")
                    .bind(own)
                    .execute(&mut *tx)
                    .await
                    .expect("set app.workspace_id");
                let all: i64 = sqlx::query_scalar(&format!("select count(*) from {table}"))
                    .fetch_one(&mut *tx)
                    .await
                    .unwrap_or_else(|e| panic!("{table}: {e}"));
                let foreign: i64 = sqlx::query_scalar(&format!(
                    "select count(*) from {table} where {column} = '{other}'::uuid"
                ))
                .fetch_one(&mut *tx)
                .await
                .unwrap_or_else(|e| panic!("{table}: {e}"));
                drop(tx);
                assert_eq!(foreign, 0, "{table}: rows of {other} visible to {own}");
                assert_eq!(
                    all,
                    rows_of(&mut admin, table, own).await,
                    "{table}: {own} sees exactly its own rows"
                );
            }
        }
    });
}

#[test]
fn without_a_workspace_set_the_api_role_sees_no_row_of_any_tenant_table() {
    let f = fixture();
    block(async {
        let tables = seeded_tenant_tables(&f.a.id, &f.b.id).await;
        let mut conn = api_connection().await;
        for table in &tables {
            let rows: i64 = sqlx::query_scalar(&format!("select count(*) from {table}"))
                .fetch_one(&mut conn)
                .await
                .unwrap_or_else(|e| panic!("{table}: {e}"));
            assert_eq!(rows, 0, "{table}: rows visible without a workspace");
        }
    });
}

#[test]
fn row_level_security_stops_workspace_a_from_changing_inserting_moving_or_deleting_rows_of_b() {
    let f = fixture();
    let _guard = serial();
    let before = (snapshot(&f.a.id), snapshot(&f.b.id));
    block(async {
        let tables = seeded_tenant_tables(&f.a.id, &f.b.id).await;
        let mut admin = admin_connection().await;
        let mut conn = api_connection().await;
        let (a, b) = (&f.a.id, &f.b.id);
        for table in &tables {
            let column = scope_column(table);

            // Updates of B's rows touch nothing: the rows are not there for A,
            // or A may not update the table at all.
            let sql =
                format!("update {table} set {column} = {column} where {column} = '{b}'::uuid");
            match attempt(&mut conn, a, &sql).await {
                Ok(rows) => assert_eq!(rows, 0, "{table}: A updated {rows} rows of B"),
                Err(code) => assert!(code.starts_with("42501"), "{table}: {code}"),
            }

            // The control for the line above: A's own rows can be updated
            // wherever the API role has the privilege.
            let may_update: bool =
                sqlx::query_scalar("select has_table_privilege('omni_api', $1, 'UPDATE')")
                    .bind(format!("public.{table}"))
                    .fetch_one(&mut admin)
                    .await
                    .expect("look up the privilege");
            if may_update {
                let sql =
                    format!("update {table} set {column} = {column} where {column} = '{a}'::uuid");
                let own = rows_of(&mut admin, table, a).await;
                match attempt(&mut conn, a, &sql).await {
                    // Append-only tables refuse every update with a trigger.
                    Err(message) => assert!(
                        message.contains("append-only"),
                        "{table}: the control update was refused: {message}"
                    ),
                    Ok(rows) => assert_eq!(rows as i64, own, "{table}: A's own rows"),
                }
            }

            // A row of A cannot be moved to B.
            let sql =
                format!("update {table} set {column} = '{b}'::uuid where {column} = '{a}'::uuid");
            let moved = attempt(&mut conn, a, &sql).await;
            assert!(moved.is_err(), "{table}: A moved rows to B: {moved:?}");

            // A copy of a row of A cannot be inserted as a row of B.
            let (workspace_key, id_key) = ("{workspace_id}", "{id}");
            let sql = if table == "workspaces" {
                format!(
                    "insert into workspaces select (jsonb_populate_record(null::workspaces, \
                     jsonb_set(to_jsonb(r), '{id_key}', to_jsonb('{b}'::text)))).* \
                     from workspaces r where r.id = '{a}'::uuid"
                )
            } else {
                format!(
                    "insert into {table} select (jsonb_populate_record(null::{table}, \
                     jsonb_set(jsonb_set(to_jsonb(r), '{workspace_key}', to_jsonb('{b}'::text)), \
                     '{id_key}', to_jsonb(uuid_v7()::text)))).* \
                     from {table} r where r.workspace_id = '{a}'::uuid limit 1"
                )
            };
            let inserted = attempt(&mut conn, a, &sql).await;
            match &inserted {
                Err(code) => assert!(code.starts_with("42501"), "{table}: {code}"),
                Ok(rows) => panic!("{table}: A inserted {rows} rows into B"),
            }

            // And no row can be deleted, whose it is.
            for owner in [a, b] {
                let sql = format!("delete from {table} where {column} = '{owner}'::uuid");
                let deleted = attempt(&mut conn, a, &sql).await;
                assert!(
                    deleted.is_err(),
                    "{table}: A deleted rows of {owner}: {deleted:?}"
                );
            }
        }
    });
    assert_eq!(
        (snapshot(&f.a.id), snapshot(&f.b.id)),
        before,
        "the attempts changed rows"
    );
}

#[test]
fn a_connection_that_set_workspace_a_for_one_transaction_has_forgotten_it_in_the_next() {
    let f = fixture();
    block(async {
        let tables = seeded_tenant_tables(&f.a.id, &f.b.id).await;
        let mut conn = api_connection().await;
        let mut tx = conn.begin().await.expect("begin");
        sqlx::query("select set_config('app.workspace_id', $1, true)")
            .bind(&f.a.id)
            .execute(&mut *tx)
            .await
            .expect("set app.workspace_id");
        let seen: i64 = sqlx::query_scalar("select count(*) from projects")
            .fetch_one(&mut *tx)
            .await
            .expect("count");
        assert!(seen > 0, "the transaction sees its workspace");
        tx.commit().await.expect("commit");
        for table in &tables {
            let rows: i64 = sqlx::query_scalar(&format!("select count(*) from {table}"))
                .fetch_one(&mut conn)
                .await
                .unwrap_or_else(|e| panic!("{table}: {e}"));
            assert_eq!(
                rows, 0,
                "{table}: the finished transaction's workspace lingers"
            );
        }
    });
}

// ---------------------------------------------------------------------------
// Docs without a change set (US-102)
// ---------------------------------------------------------------------------

const LIVE_DOC_ROUTES: [&str; 2] = [
    "/api/v1/w/{workspace_id}/projects/{project_id}/docs",
    "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/docs",
];

#[test]
fn the_scope_level_doc_routes_are_in_the_registry_and_the_suite_attacks_them() {
    for path in LIVE_DOC_ROUTES {
        let entry = registry()
            .into_iter()
            .find(|r| r.method == "POST" && r.path == path)
            .unwrap_or_else(|| panic!("the registry has no route POST {path}"));
        assert_eq!(entry.access, Access::Agent, "POST {path}");
        assert!(
            matches!(case_for(&entry).kind, Kind::Workspace),
            "POST {path} is a workspace route that the suite attacks"
        );
    }

    // The suite seeded the log of the writes to docs for both workspaces.
    let (f, _) = ready();
    for side in [&f.a, &f.b] {
        let rows = block(async {
            let mut admin = admin_connection().await;
            rows_of(&mut admin, "doc_changes", &side.id).await
        });
        assert!(rows > 0, "the suite seeded no doc change for {}", side.id);
    }
}

#[test]
fn the_scope_level_doc_calls_of_the_attacks_work_for_the_workspace_that_owns_the_records() {
    // The control for the scope-level doc routes: in the owner's workspace,
    // the owner's agent creates a doc with the body of each case, and both the
    // agent and the human read it back in the scope it was created in.
    let (f, client) = ready();
    let _guard = serial();
    let key = Auth::Key(&f.a.key);
    let human = Auth::Session(&f.a.token);
    let project = format!("/api/v1/w/{}/projects/{}", f.a.id, f.a.project_id);
    for (path, feature) in [
        (LIVE_DOC_ROUTES[0], Value::Null),
        (LIVE_DOC_ROUTES[1], json!(f.a.feature_id)),
    ] {
        let title = unique("owner");
        let body = case_named("POST", path)
            .body
            .map(|b| b.replace("intruder", &title));
        let created = must(
            call(
                client,
                "POST",
                &fill(path, &f.a, &f.a),
                &key,
                body.as_deref(),
            ),
            201,
            "create a doc without a change set",
        );
        let doc = created["doc"]["id"].as_str().expect("an id");
        assert_eq!(created["doc"]["feature_id"], feature, "{created}");
        for auth in [&key, &human] {
            let read = must(
                call(client, "GET", &format!("{project}/docs/{doc}"), auth, None),
                200,
                "read the doc",
            );
            assert_eq!(read["doc"]["title"], title.as_str(), "{read}");
            assert_eq!(read["doc"]["feature_id"], feature, "{read}");
        }
    }
}

// ---------------------------------------------------------------------------
// The activity of a doc (US-111)
// ---------------------------------------------------------------------------

const ACTIVITY_ROUTES: [&str; 2] = [
    "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/activity",
    "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/activity/{seq}",
];

#[test]
fn the_activity_routes_are_in_the_registry_and_the_suite_attacks_them() {
    for path in ACTIVITY_ROUTES {
        let entry = registry()
            .into_iter()
            .find(|r| r.method == "GET" && r.path == path)
            .unwrap_or_else(|| panic!("the registry has no route GET {path}"));
        assert_eq!(entry.access, Access::Both, "GET {path}");
        assert!(
            matches!(case_for(&entry).kind, Kind::Workspace),
            "GET {path} is a workspace route that the suite attacks"
        );
    }
}

#[test]
fn the_activity_calls_of_the_attacks_work_for_the_workspace_that_owns_the_records() {
    // The control for the activity routes: in the owner's workspace, the
    // agent and the human read the activity of the seeded doc and of a doc
    // the owner's agent creates and writes, and its first write.
    let (f, client) = ready();
    let _guard = serial();
    let key = Auth::Key(&f.a.key);
    let human = Auth::Session(&f.a.token);
    let project = format!("/api/v1/w/{}/projects/{}", f.a.id, f.a.project_id);
    let created = must(
        call(
            client,
            "POST",
            &format!("{project}/docs"),
            &key,
            Some(
                &json!({ "kind": "html", "pillar": "spec", "title": unique("owner-doc") })
                    .to_string(),
            ),
        ),
        201,
        "create a doc without a change set",
    );
    let doc = created["doc"]["id"].as_str().expect("an id").to_string();
    must(
        call(
            client,
            "POST",
            &format!("{project}/docs/{doc}/ops"),
            &key,
            Some(
                r#"{"ops":[{"op":"insert","parent":null,"position":{"after":null},"content":"<p>owner</p>"}]}"#,
            ),
        ),
        200,
        "write the doc",
    );
    for auth in [&key, &human] {
        let list = must(
            call(
                client,
                "GET",
                &format!("{project}/docs/{doc}/activity"),
                auth,
                None,
            ),
            200,
            "read the doc's activity",
        );
        let seqs: Vec<i64> = list["entries"]
            .as_array()
            .unwrap_or_else(|| panic!("the activity lists entries: {list}"))
            .iter()
            .map(|entry| entry["seq"].as_i64().expect("a seq"))
            .collect();
        assert_eq!(seqs, [2, 1], "{list}");
        let first = must(
            call(
                client,
                "GET",
                &format!("{project}/docs/{doc}/activity/1"),
                auth,
                None,
            ),
            200,
            "read the doc's first write",
        );
        assert_eq!(first["entry"]["seq"], 1, "{first}");
        for path in ACTIVITY_ROUTES {
            must(
                call(client, "GET", &fill(path, &f.a, &f.a), auth, None),
                200,
                "read the activity of the seeded doc",
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Threads without a change set (US-108)
// ---------------------------------------------------------------------------

const LIVE_THREAD_ROUTES: [&str; 2] = [
    "/api/v1/w/{workspace_id}/projects/{project_id}/threads",
    "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/threads",
];
const CONCLUSION_ROUTE: &str =
    "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/conclusion";

#[test]
fn the_scope_level_thread_routes_are_in_the_registry_and_the_suite_attacks_them() {
    let routes = registry();
    for (method, path, access) in [
        ("POST", LIVE_THREAD_ROUTES[0], Access::Agent),
        ("POST", LIVE_THREAD_ROUTES[1], Access::Agent),
        ("GET", LIVE_THREAD_ROUTES[0], Access::Both),
        ("PUT", CONCLUSION_ROUTE, Access::Agent),
    ] {
        let entry = routes
            .iter()
            .find(|r| r.method == method && r.path == path)
            .unwrap_or_else(|| panic!("the registry has no route {method} {path}"));
        assert_eq!(entry.access, access, "{method} {path}");
        assert!(
            matches!(case_for(entry).kind, Kind::Workspace),
            "{method} {path} is a workspace route that the suite attacks"
        );
    }
}

#[test]
fn the_scope_level_thread_calls_of_the_attacks_work_for_the_workspace_that_owns_the_records() {
    // The control for the thread routes without a change set: in the owner's
    // workspace, the owner's agent starts a thread with the body of each case,
    // both the agent and the human find it in the list and read it, and the
    // agent concludes it with the body of the conclusion's case.
    let (f, client) = ready();
    let _guard = serial();
    let key = Auth::Key(&f.a.key);
    let human = Auth::Session(&f.a.token);
    let project = format!("/api/v1/w/{}/projects/{}", f.a.id, f.a.project_id);
    for (path, feature) in [
        (LIVE_THREAD_ROUTES[0], Value::Null),
        (LIVE_THREAD_ROUTES[1], json!(f.a.feature_id)),
    ] {
        let question = unique("owner");
        let body = case_named("POST", path)
            .body
            .map(|b| b.replace("intruder", &question));
        let created = must(
            call(
                client,
                "POST",
                &fill(path, &f.a, &f.a),
                &key,
                body.as_deref(),
            ),
            201,
            "start a thread without a change set",
        );
        let thread = created["thread"]["id"].as_str().expect("an id");
        assert_eq!(created["thread"]["feature_id"], feature, "{created}");
        for auth in [&key, &human] {
            let listed = must(
                call(client, "GET", &format!("{project}/threads"), auth, None),
                200,
                "list the threads",
            );
            assert!(
                listed["threads"]
                    .as_array()
                    .is_some_and(|threads| threads.iter().any(|t| t["id"] == thread)),
                "the list has the new thread: {listed}"
            );
            let read = must(
                call(
                    client,
                    "GET",
                    &format!("{project}/threads/{thread}"),
                    auth,
                    None,
                ),
                200,
                "read the thread",
            );
            assert_eq!(read["thread"]["question"], question.as_str(), "{read}");
        }
        let conclusion = unique("concluded");
        let body = case_named("PUT", CONCLUSION_ROUTE)
            .body
            .map(|b| b.replace("intruder", &conclusion));
        let concluded = must(
            call(
                client,
                "PUT",
                &format!("{project}/threads/{thread}/conclusion"),
                &key,
                body.as_deref(),
            ),
            200,
            "conclude the thread",
        );
        assert_eq!(
            concluded["thread"]["conclusion"],
            conclusion.as_str(),
            "{concluded}"
        );
    }
}
