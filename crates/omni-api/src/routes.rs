//! The route registry: every route of the API with who may call it.

/// Who may call a route: agents (API keys), the human (a browser session), or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Agent,
    Human,
    Both,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteEntry {
    /// The HTTP method in upper case, such as `GET`.
    pub method: String,
    /// The path in OpenAPI form, such as `/api/v1/w/{workspace_id}/projects`.
    pub path: String,
    pub access: Access,
}

/// Every WebSocket route that the router serves. It is kept apart from
/// `registry`, which lists the HTTP routes of the OpenAPI document: a
/// WebSocket route is no OpenAPI operation, but the isolation suite attacks it
/// like the rest (tests/isolation.rs fails on one that it does not cover).
pub fn websocket_registry() -> Vec<RouteEntry> {
    // US-018 (green): list `GET /ws/w/{workspace_id}`, which is open to agents
    // and to the human.
    Vec::new()
}

/// Every HTTP route that the router serves. A new route is added here and in
/// `router`; the OpenAPI document picks the access up from this list.
pub fn registry() -> Vec<RouteEntry> {
    [
        ("GET", "/api/health", Access::Both),
        ("GET", "/api/openapi.json", Access::Both),
        ("POST", "/api/v1/auth/login", Access::Human),
        ("POST", "/api/v1/auth/logout", Access::Human),
        ("GET", "/api/v1/auth/me", Access::Human),
        ("GET", "/api/v1/auth/workspaces", Access::Human),
        ("GET", "/api/v1/w/{workspace_id}/whoami", Access::Both),
        ("GET", "/api/v1/w/{workspace_id}/api-keys", Access::Human),
        ("POST", "/api/v1/w/{workspace_id}/api-keys", Access::Human),
        (
            "POST",
            "/api/v1/w/{workspace_id}/api-keys/{key_id}/revoke",
            Access::Human,
        ),
        ("GET", "/api/v1/w/{workspace_id}/projects", Access::Both),
        ("POST", "/api/v1/w/{workspace_id}/projects", Access::Agent),
        (
            "GET",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets",
            Access::Both,
        ),
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets",
            Access::Agent,
        ),
        (
            "GET",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}",
            Access::Both,
        ),
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/submit",
            Access::Agent,
        ),
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/reject",
            Access::Human,
        ),
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/reopen",
            Access::Agent,
        ),
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/abandon",
            Access::Agent,
        ),
    ]
    .into_iter()
    .map(|(method, path, access)| RouteEntry {
        method: method.to_string(),
        path: path.to_string(),
        access,
    })
    .collect()
}
