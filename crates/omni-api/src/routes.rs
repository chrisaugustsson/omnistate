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

/// Every HTTP route that the router serves.
pub fn registry() -> Vec<RouteEntry> {
    todo!("the route registry")
}
