//! US-012: the running Compose stack, as seen from the test container through
//! the reverse proxy (OMNI_STACK_URL, default http://proxy). Run with
//! `--features stack-tests`. The stack's database has no user unless someone
//! ran `omni-api setup` on it, so these tests only use requests that fail the
//! same way either way.

mod common;

use common::{Client, cookie};
use serde_json::json;

#[test]
fn the_openapi_document_is_served_through_the_proxy() {
    let response = Client::proxy().get("/api/openapi.json");
    assert_eq!(response.status, 200, "GET /api/openapi.json");
    assert!(
        response.content_type().starts_with("application/json"),
        "the API answers, not the web app: {}",
        response.content_type()
    );
    assert!(
        response.json()["openapi"]
            .as_str()
            .is_some_and(|v| v.starts_with("3.")),
        "an OpenAPI 3 document"
    );
}

#[test]
fn login_with_bad_credentials_is_401_json_from_the_running_api() {
    let response = Client::proxy().post_json(
        "/api/v1/auth/login",
        &json!({ "email": "nobody@example.test", "password": "not a password" }),
        &[],
    );
    assert_eq!(response.status, 401, "login body: {}", response.text());
    assert_eq!(response.json()["error"], "invalid_credentials");
    assert!(
        response.session_set_cookie().is_none(),
        "no session cookie for bad credentials"
    );
}

#[test]
fn me_without_a_valid_session_is_401_from_the_running_api() {
    let client = Client::proxy();
    let anonymous = client.get("/api/v1/auth/me");
    assert_eq!(anonymous.status, 401);
    assert_eq!(anonymous.json()["error"], "unauthorized");

    let forged = client.get_with(
        "/api/v1/auth/me",
        &[("Cookie", &cookie("0123456789abcdef0123456789abcdef"))],
    );
    assert_eq!(forged.status, 401, "a token that was never issued");
}
