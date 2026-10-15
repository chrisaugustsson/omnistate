//! US-012: the OpenAPI document and the route registry. Every route of the API
//! is in the registry marked `agent`, `human` or `both`, and the generated
//! OpenAPI document at `GET /api/openapi.json` describes exactly those routes,
//! each operation carrying its registry access in `x-omni-access`.

mod common;

use common::{TestServer, offline_config};
use omni_api::routes::{Access, RouteEntry, registry};
use serde_json::Value;
use std::collections::BTreeSet;

const HTTP_METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "patch", "head", "options", "trace",
];

fn access_name(access: Access) -> &'static str {
    match access {
        Access::Agent => "agent",
        Access::Human => "human",
        Access::Both => "both",
    }
}

fn find<'a>(routes: &'a [RouteEntry], method: &str, path: &str) -> Option<&'a RouteEntry> {
    routes.iter().find(|r| r.method == method && r.path == path)
}

fn openapi(server: &TestServer) -> Value {
    let response = server.client.get("/api/openapi.json");
    assert_eq!(response.status, 200, "GET /api/openapi.json");
    assert!(
        response.content_type().starts_with("application/json"),
        "the OpenAPI document is JSON: {}",
        response.content_type()
    );
    response.json()
}

/// (METHOD, path, access) of every operation in the document.
fn operations(document: &Value) -> Vec<(String, String, Option<String>)> {
    let paths = document["paths"]
        .as_object()
        .unwrap_or_else(|| panic!("the document has no paths object: {document}"));
    let mut found = Vec::new();
    for (path, item) in paths {
        for (key, operation) in item.as_object().expect("path item") {
            if HTTP_METHODS.contains(&key.as_str()) {
                found.push((
                    key.to_uppercase(),
                    path.clone(),
                    operation["x-omni-access"].as_str().map(str::to_string),
                ));
            }
        }
    }
    found
}

#[test]
fn the_api_serves_an_openapi_3_document_at_api_openapi_json() {
    let server = TestServer::start(&offline_config());
    let document = openapi(&server);
    let version = document["openapi"].as_str().unwrap_or("");
    assert!(
        version.starts_with("3."),
        "`openapi` must be a 3.x version: {version:?}"
    );
    assert!(
        document["info"]["title"]
            .as_str()
            .is_some_and(|t| !t.is_empty()),
        "info.title: {}",
        document["info"]
    );
    assert!(
        document["info"]["version"]
            .as_str()
            .is_some_and(|t| !t.is_empty()),
        "info.version: {}",
        document["info"]
    );
}

#[test]
fn the_registry_lists_the_routes_of_this_story_with_who_may_call_them() {
    let routes = registry();
    assert!(!routes.is_empty(), "the route registry is empty");

    let expected = [
        ("GET", "/api/health", Access::Both),
        ("GET", "/api/openapi.json", Access::Both),
        ("POST", "/api/v1/auth/login", Access::Human),
        ("POST", "/api/v1/auth/logout", Access::Human),
        ("GET", "/api/v1/auth/me", Access::Human),
    ];
    for (method, path, access) in expected {
        let entry = find(&routes, method, path)
            .unwrap_or_else(|| panic!("the registry has no entry for {method} {path}: {routes:?}"));
        assert_eq!(entry.access, access, "{method} {path}");
    }

    let unique: BTreeSet<_> = routes.iter().map(|r| (&r.method, &r.path)).collect();
    assert_eq!(unique.len(), routes.len(), "a route is registered twice");
    for route in &routes {
        assert_eq!(
            route.method,
            route.method.to_uppercase(),
            "methods are upper case: {route:?}"
        );
        assert!(
            route.path.starts_with("/api/"),
            "registered paths are the full API paths: {route:?}"
        );
    }
}

#[test]
fn every_registered_route_is_in_the_openapi_document_with_its_access() {
    let server = TestServer::start(&offline_config());
    let document = openapi(&server);
    let described = operations(&document);
    for route in registry() {
        let entry = described
            .iter()
            .find(|(method, path, _)| *method == route.method && *path == route.path)
            .unwrap_or_else(|| {
                panic!(
                    "{} {} is registered but not in the OpenAPI document: {described:?}",
                    route.method, route.path
                )
            });
        assert_eq!(
            entry.2.as_deref(),
            Some(access_name(route.access)),
            "x-omni-access of {} {}",
            route.method,
            route.path
        );
    }
}

#[test]
fn every_operation_in_the_openapi_document_is_in_the_registry() {
    let server = TestServer::start(&offline_config());
    let document = openapi(&server);
    let routes = registry();
    let described = operations(&document);
    assert!(!described.is_empty(), "the document describes no operation");
    for (method, path, access) in described {
        assert!(
            find(&routes, &method, &path).is_some(),
            "{method} {path} is in the OpenAPI document but not in the route registry"
        );
        assert!(
            matches!(access.as_deref(), Some("agent" | "human" | "both")),
            "{method} {path} needs x-omni-access agent, human or both: {access:?}"
        );
    }
}

#[test]
fn every_registered_route_is_served_by_the_router() {
    let server = TestServer::start(&offline_config());
    for route in registry() {
        let path = route
            .path
            .replace("{workspace_id}", "00000000-0000-7000-8000-000000000000");
        let response = server.client.request(&route.method, &path, &[], None);
        let fallback = response.status == 404
            && response.content_type().starts_with("application/json")
            && response.json()["error"] == "not_found";
        assert!(
            !fallback,
            "{} {} is registered but the router answers it with the not-found fallback",
            route.method, route.path
        );
        assert_ne!(
            response.status, 405,
            "{} {} is registered but the router does not serve that method",
            route.method, route.path
        );
    }
}

#[test]
fn the_document_describes_the_login_operation_from_its_handler() {
    let server = TestServer::start(&offline_config());
    let document = openapi(&server);
    let login = &document["paths"]["/api/v1/auth/login"]["post"];
    assert!(
        login.is_object(),
        "the document has no POST /api/v1/auth/login: {}",
        document["paths"]
    );
    assert!(
        login["requestBody"].is_object(),
        "login describes its request body: {login}"
    );
    let responses = login["responses"].as_object().expect("login responses");
    for status in ["200", "401"] {
        assert!(
            responses.contains_key(status),
            "login documents its {status} response: {login}"
        );
    }
}
