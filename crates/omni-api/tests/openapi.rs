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
fn the_registry_and_the_openapi_document_list_the_same_routes_with_the_same_access() {
    let server = TestServer::start(&offline_config());
    let document = openapi(&server);
    let registered: BTreeSet<(String, String, Option<String>)> = registry()
        .into_iter()
        .map(|route| {
            (
                route.method,
                route.path,
                Some(access_name(route.access).to_string()),
            )
        })
        .collect();
    assert!(!registered.is_empty(), "the route registry is empty");
    let described: BTreeSet<(String, String, Option<String>)> =
        operations(&document).into_iter().collect();

    let only_registered: Vec<_> = registered.difference(&described).collect();
    assert!(
        only_registered.is_empty(),
        "registered but not in the OpenAPI document with that x-omni-access: {only_registered:?}"
    );
    let only_described: Vec<_> = described.difference(&registered).collect();
    assert!(
        only_described.is_empty(),
        "in the OpenAPI document but not in the route registry with that access: {only_described:?}"
    );
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

/// Every live event the API emits, by name (`<entity>.<action>`).
const LIVE_EVENTS: [&str; 14] = [
    "comment.created",
    "comment.deleted",
    "comment.replied",
    "doc.changed",
    "doc.created",
    "doc.deleted",
    "doc.renamed",
    "feature.created",
    "feature.deleted",
    "feature.renamed",
    "post.created",
    "project.renamed",
    "thread.concluded",
    "thread.created",
];

/// Every string that a schema (following `$ref`s into the document's
/// components) allows as an `enum` member or a `const`.
fn allowed_strings(
    document: &Value,
    schema: &Value,
    seen: &mut BTreeSet<String>,
) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    match schema {
        Value::Object(map) => {
            if let Some(reference) = map.get("$ref").and_then(Value::as_str)
                && seen.insert(reference.to_string())
            {
                let target = document
                    .pointer(reference.trim_start_matches('#'))
                    .unwrap_or_else(|| panic!("{reference} is not in the document"));
                found.extend(allowed_strings(document, target, seen));
            }
            if let Some(Value::String(constant)) = map.get("const") {
                found.insert(constant.clone());
            }
            if let Some(Value::Array(members)) = map.get("enum") {
                found.extend(members.iter().filter_map(Value::as_str).map(str::to_string));
            }
            for (key, value) in map {
                if !matches!(key.as_str(), "$ref" | "const" | "enum") {
                    found.extend(allowed_strings(document, value, seen));
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                found.extend(allowed_strings(document, item, seen));
            }
        }
        _ => {}
    }
    found
}

fn component<'a>(document: &'a Value, name: &str) -> &'a Value {
    let schema = &document["components"]["schemas"][name];
    assert!(
        schema.is_object(),
        "the OpenAPI document has no component schema {name}, which the web app's generated types need"
    );
    schema
}

// US-039: the web app generates its types for the live updates from the
// document too, so the messages of the socket are schemas of the document:
// `LiveClientMessage` (what a client sends: a subscribe) and
// `LiveServerMessage` (what the server sends: subscribed, event, error), and
// the event message names every event the API emits.
#[test]
fn the_document_describes_the_messages_of_the_live_socket() {
    let server = TestServer::start(&offline_config());
    let document = openapi(&server);

    let client = component(&document, "LiveClientMessage");
    let client_strings = allowed_strings(&document, client, &mut BTreeSet::new());
    assert!(
        client_strings.contains("subscribe"),
        "LiveClientMessage allows the type `subscribe`: {client}"
    );

    let server_message = component(&document, "LiveServerMessage");
    let strings = allowed_strings(&document, server_message, &mut BTreeSet::new());
    for message_type in ["subscribed", "event", "error"] {
        assert!(
            strings.contains(message_type),
            "LiveServerMessage allows the type `{message_type}`: {server_message}"
        );
    }
    let missing: Vec<_> = LIVE_EVENTS
        .iter()
        .filter(|event| !strings.contains(**event))
        .collect();
    assert!(
        missing.is_empty(),
        "LiveServerMessage names every live event the API emits; missing: {missing:?}"
    );
}

/// The copy of the OpenAPI document that the web app generates its TypeScript
/// types and Zod schemas from.
fn web_document_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/openapi.json")
}

// US-039: the web app's copy of the document is the one the API serves, so a
// change to the API that is not regenerated into the web app fails the build.
#[test]
fn the_web_app_has_the_current_openapi_document() {
    let server = TestServer::start(&offline_config());
    let document = openapi(&server);
    let path = web_document_path();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "the web app's copy of the OpenAPI document, {}, cannot be read: {error}",
            path.display()
        )
    });
    let copy: Value = serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{} is not JSON: {error}", path.display()));
    assert!(
        copy == document,
        "{} is out of date: regenerate it and the web app's types from the API's document",
        path.display()
    );
}

// US-061: a doc belongs to the project or one of its features, so the doc
// body carries `feature_id`, and the project's docs are listed with the
// `feature` filter.
#[test]
fn the_document_describes_the_feature_of_docs_and_the_doc_list() {
    let docs = "/api/v1/w/{workspace_id}/projects/{project_id}/docs";
    let routes = registry();
    let entry = find(&routes, "GET", docs)
        .unwrap_or_else(|| panic!("the registry has no entry for GET {docs}"));
    assert_eq!(entry.access, Access::Both, "GET {docs}");

    let server = TestServer::start(&offline_config());
    let document = openapi(&server);
    let body = component(&document, "DocBody");
    assert!(
        body["properties"]["feature_id"].is_object(),
        "DocBody has the field feature_id: {body}"
    );
    let list = component(&document, "DocListResponse");
    assert!(
        list["properties"]["docs"].is_object(),
        "DocListResponse has the field docs: {list}"
    );
    let get = &document["paths"][docs]["get"];
    let parameters = get["parameters"].as_array().cloned().unwrap_or_default();
    assert!(
        parameters
            .iter()
            .any(|p| p["name"] == "feature" && p["in"] == "query"),
        "GET .../docs documents the query parameter `feature`: {get}"
    );
}

// US-111: the activity of a doc is paged with `limit` and `before`, and a
// board and a component read as a write left them with `at`.
#[test]
fn the_document_describes_the_activity_routes_and_the_reads_at_a_write() {
    let list = "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/activity";
    let board = "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/boards/{board_id}";
    let component = "/api/v1/w/{workspace_id}/projects/{project_id}/components/{component_id}";
    let server = TestServer::start(&offline_config());
    let document = openapi(&server);
    let query_names = |path: &str| -> Vec<String> {
        let get = &document["paths"][path]["get"];
        get["parameters"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter(|p| p["in"] == "query")
            .filter_map(|p| p["name"].as_str().map(str::to_string))
            .collect()
    };
    for name in ["limit", "before"] {
        assert!(
            query_names(list).iter().any(|n| n == name),
            "GET {list} documents the query parameter `{name}`"
        );
    }
    assert_eq!(query_names(component), ["at"], "GET {component}");
    assert!(
        query_names(board).iter().any(|n| n == "at"),
        "GET {board} documents the query parameter `at`"
    );
    let text = document.to_string();
    for field in ["entries", "next_before", "kind", "summary", "items"] {
        assert!(
            text.contains(&format!("\"{field}\"")),
            "the document describes the field `{field}` of the activity"
        );
    }
}
