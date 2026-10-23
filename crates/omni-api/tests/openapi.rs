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

/// Every live event the API emits, by name (`<entity>.<action>`): the
/// `changeset.<status>` events of every status and the events of US-017 on.
const LIVE_EVENTS: [&str; 34] = [
    "changeset.open",
    "changeset.in_review",
    "changeset.approved",
    "changeset.rejected",
    "changeset.merged",
    "changeset.needs_rebase",
    "changeset.abandoned",
    "claim.taken",
    "claim.released",
    "claim.expired",
    "comment.created",
    "comment.replied",
    "comment.status_changed",
    "components.imported",
    "conflict.recorded",
    "conflict.resolved",
    "decision.proposed",
    "decision.superseded",
    "decision.approved",
    "decision.rejected",
    "design.imported",
    "doc.changed",
    "doc.created",
    "draft.opened",
    "draft.rebased",
    "feature.created",
    "feature.done",
    "feature.reopened",
    "op.applied",
    "phase.started",
    "post.created",
    "thread.created",
    "tokens.imported",
    "version.saved",
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

// US-059: the feature routes are in the registry and the document, and the
// bodies they answer with are schemas of the document, from which the web app
// generates its types.
#[test]
fn the_document_describes_the_feature_routes_and_their_bodies() {
    let routes = registry();
    let features = "/api/v1/w/{workspace_id}/projects/{project_id}/features";
    let feature = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}";
    for (method, path, access) in [
        ("POST", features, Access::Agent),
        ("GET", features, Access::Both),
        ("GET", feature, Access::Both),
    ] {
        let entry = find(&routes, method, path)
            .unwrap_or_else(|| panic!("the registry has no entry for {method} {path}"));
        assert_eq!(entry.access, access, "{method} {path}");
    }

    let server = TestServer::start(&offline_config());
    let document = openapi(&server);
    for name in [
        "FeatureBody",
        "CreateFeatureRequest",
        "FeatureResponse",
        "FeatureListResponse",
    ] {
        component(&document, name);
    }
    let body = component(&document, "FeatureBody");
    for field in [
        "id",
        "project_id",
        "name",
        "description",
        "status",
        "created_at",
    ] {
        assert!(
            body["properties"][field].is_object(),
            "FeatureBody has the field {field}: {body}"
        );
    }
}

// US-060: a change set belongs to the project or one of its features, so the
// change set and thread bodies carry `feature_id`, the create request takes
// it, and the change set list takes the `feature` filter.
#[test]
fn the_document_describes_the_feature_of_change_sets_and_threads() {
    let server = TestServer::start(&offline_config());
    let document = openapi(&server);
    for name in ["ChangesetBody", "ThreadBody", "CreateChangesetRequest"] {
        let schema = component(&document, name);
        assert!(
            schema["properties"]["feature_id"].is_object(),
            "{name} has the field feature_id: {schema}"
        );
    }
    let list =
        &document["paths"]["/api/v1/w/{workspace_id}/projects/{project_id}/changesets"]["get"];
    let parameters = list["parameters"].as_array().cloned().unwrap_or_default();
    assert!(
        parameters
            .iter()
            .any(|p| p["name"] == "feature" && p["in"] == "query"),
        "GET .../changesets documents the query parameter `feature`: {list}"
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

// US-101: every doc sits in one of five pillars, so the doc body and the
// create request carry `pillar` instead of the old `module`, and the doc list
// takes the `pillar` filter.
#[test]
fn the_document_describes_the_pillar_of_docs_and_the_pillar_filter() {
    let server = TestServer::start(&offline_config());
    let document = openapi(&server);
    for name in ["DocBody", "CreateDocRequest"] {
        let schema = component(&document, name);
        assert!(
            schema["properties"]["pillar"].is_object(),
            "{name} has the field pillar: {schema}"
        );
        assert!(
            schema["properties"].get("module").is_none(),
            "{name} has no field module any more: {schema}"
        );
    }
    let body = component(&document, "DocBody");
    let required = body["required"].as_array().cloned().unwrap_or_default();
    assert!(
        required.iter().any(|field| field == "pillar"),
        "every doc has a pillar: {body}"
    );
    let docs = "/api/v1/w/{workspace_id}/projects/{project_id}/docs";
    let get = &document["paths"][docs]["get"];
    let parameters = get["parameters"].as_array().cloned().unwrap_or_default();
    assert!(
        parameters
            .iter()
            .any(|p| p["name"] == "pillar" && p["in"] == "query"),
        "GET .../docs documents the query parameter `pillar`: {get}"
    );
}

// US-062: the human marks a feature done or reopens it, through routes of the
// registry and the document that answer with the feature.
#[test]
fn the_document_describes_the_routes_that_finish_and_reopen_a_feature() {
    let routes = registry();
    let server = TestServer::start(&offline_config());
    let document = openapi(&server);
    for action in ["done", "reopen"] {
        let path = format!(
            "/api/v1/w/{{workspace_id}}/projects/{{project_id}}/features/{{feature_id}}/{action}"
        );
        let entry = find(&routes, "POST", &path)
            .unwrap_or_else(|| panic!("the registry has no entry for POST {path}"));
        assert_eq!(entry.access, Access::Human, "POST {path}");
        let operation = &document["paths"][path.as_str()]["post"];
        assert!(operation.is_object(), "the document describes POST {path}");
        let answer = operation["responses"]["200"].to_string();
        assert!(
            answer.contains("FeatureResponse"),
            "POST {path} answers with a FeatureResponse: {operation}"
        );
    }
}

// US-063: the project home lists the features with their counts, and every
// item that waits for the human carries the feature it belongs to.
#[test]
fn the_document_describes_the_features_of_the_project_home() {
    let server = TestServer::start(&offline_config());
    let document = openapi(&server);
    let home = component(&document, "HomeResponse");
    assert!(
        home["properties"]["features"]
            .to_string()
            .contains("FeatureSummary"),
        "HomeResponse has the field features, a list of FeatureSummary: {home}"
    );
    let summary = component(&document, "FeatureSummary");
    for field in ["feature", "docs", "open_changesets", "waiting"] {
        assert!(
            summary["properties"][field].is_object(),
            "FeatureSummary has the field {field}: {summary}"
        );
    }
    assert!(
        summary["properties"]["feature"]
            .to_string()
            .contains("FeatureBody"),
        "FeatureSummary.feature is a FeatureBody: {summary}"
    );
    for name in ["DecisionBody", "ChangesetBody", "CommentBody", "PostBody"] {
        let schema = component(&document, name);
        assert!(
            schema["properties"]["feature_id"].is_object(),
            "{name} has the field feature_id: {schema}"
        );
    }
}
