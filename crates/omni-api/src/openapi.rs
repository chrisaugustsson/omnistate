//! The OpenAPI document, generated from the handlers' `#[utoipa::path]`
//! attributes. Each operation gets an `x-omni-access` extension with its entry
//! in the route registry (`agent`, `human` or `both`), so the document and the
//! registry cannot disagree about who may call a route.

use crate::routes::{Access, registry};
use crate::session::{ErrorBody, LoginRequest, UserBody, UserResponse};
use axum::Json;
use serde_json::{Value, json};
use std::sync::OnceLock;
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "OmniState API",
        description = "The HTTP API behind the OmniState web app and the `omni` command line."
    ),
    paths(
        crate::health,
        document,
        crate::session::login,
        crate::session::logout,
        crate::session::me
    ),
    components(schemas(LoginRequest, UserBody, UserResponse, ErrorBody))
)]
struct ApiDoc;

fn access_name(access: Access) -> &'static str {
    match access {
        Access::Agent => "agent",
        Access::Human => "human",
        Access::Both => "both",
    }
}

fn build() -> Value {
    let mut document = serde_json::to_value(ApiDoc::openapi()).expect("the OpenAPI document");
    for route in registry() {
        let operation = document
            .pointer_mut(&format!(
                "/paths/{}/{}",
                route.path.replace('~', "~0").replace('/', "~1"),
                route.method.to_lowercase()
            ))
            .and_then(Value::as_object_mut);
        if let Some(operation) = operation {
            operation.insert(
                "x-omni-access".to_string(),
                json!(access_name(route.access)),
            );
        }
    }
    document
}

/// The OpenAPI document of the API.
#[utoipa::path(
    get,
    path = "/api/openapi.json",
    responses((status = 200, description = "The OpenAPI 3 document", content_type = "application/json"))
)]
pub async fn document() -> Json<Value> {
    static DOCUMENT: OnceLock<Value> = OnceLock::new();
    Json(DOCUMENT.get_or_init(build).clone())
}
