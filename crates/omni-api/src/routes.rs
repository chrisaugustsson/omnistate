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
    vec![RouteEntry {
        method: "GET".to_string(),
        path: "/ws/w/{workspace_id}".to_string(),
        access: Access::Both,
    }]
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
            "/api/v1/w/{workspace_id}/projects/{project_id}/features",
            Access::Both,
        ),
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/features",
            Access::Agent,
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
        (
            "GET",
            "/api/v1/w/{workspace_id}/projects/{project_id}/home",
            Access::Human,
        ),
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
            "GET",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/review",
            Access::Human,
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
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/approve",
            Access::Human,
        ),
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
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs",
            Access::Agent,
        ),
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/docs",
            Access::Agent,
        ),
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/docs",
            Access::Agent,
        ),
        (
            "GET",
            "/api/v1/w/{workspace_id}/projects/{project_id}/docs",
            Access::Both,
        ),
        (
            "GET",
            "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}",
            Access::Both,
        ),
        (
            "PATCH",
            "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}",
            Access::Agent,
        ),
        (
            "DELETE",
            "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}",
            Access::Agent,
        ),
        (
            "GET",
            "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/html",
            Access::Both,
        ),
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/ops",
            Access::Agent,
        ),
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/open",
            Access::Agent,
        ),
        (
            "GET",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}",
            Access::Both,
        ),
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/ops",
            Access::Agent,
        ),
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/claims",
            Access::Agent,
        ),
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/claims/release",
            Access::Agent,
        ),
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/versions",
            Access::Agent,
        ),
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/remove",
            Access::Agent,
        ),
        (
            "GET",
            "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/versions",
            Access::Both,
        ),
        (
            "GET",
            "/api/v1/w/{workspace_id}/projects/{project_id}/versions/{version_id}",
            Access::Both,
        ),
        (
            "GET",
            "/api/v1/w/{workspace_id}/projects/{project_id}/versions/{version_id}/diff/{other_version_id}",
            Access::Both,
        ),
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
        (
            "PUT",
            "/api/v1/w/{workspace_id}/projects/{project_id}/tokens",
            Access::Agent,
        ),
        (
            "PUT",
            "/api/v1/w/{workspace_id}/projects/{project_id}/components",
            Access::Agent,
        ),
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
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/boards/{board_id}",
            Access::Both,
        ),
        (
            "GET",
            "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/boards/{board_id}",
            Access::Both,
        ),
        (
            "GET",
            "/api/v1/w/{workspace_id}/projects/{project_id}/components/usage",
            Access::Both,
        ),
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
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/threads",
            Access::Agent,
        ),
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/threads",
            Access::Agent,
        ),
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/threads",
            Access::Agent,
        ),
        (
            "GET",
            "/api/v1/w/{workspace_id}/projects/{project_id}/threads",
            Access::Both,
        ),
        (
            "PUT",
            "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/conclusion",
            Access::Agent,
        ),
        (
            "GET",
            "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}",
            Access::Both,
        ),
        (
            "GET",
            "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/digest",
            Access::Both,
        ),
        (
            "GET",
            "/api/v1/w/{workspace_id}/projects/{project_id}/posts/{post_id}",
            Access::Both,
        ),
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/phases",
            Access::Agent,
        ),
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/posts",
            Access::Both,
        ),
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/directives",
            Access::Human,
        ),
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
    ]
    .into_iter()
    .map(|(method, path, access)| RouteEntry {
        method: method.to_string(),
        path: path.to_string(),
        access,
    })
    .collect()
}
