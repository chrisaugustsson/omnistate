//! US-012: all authentication goes through the `AuthProvider` trait. These
//! tests serve the API with a fake provider and no reachable database, so
//! nothing but the trait can be deciding who is logged in: the login, the
//! session cookie, `me` and the logout must all be driven by the fake.

mod common;

use common::{SESSION_COOKIE, TestServer, cookie, offline_config};
use omni_api::auth::{AuthError, AuthProvider, AuthUser, BoxFuture, Session};
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

const FAKE_EMAIL: &str = "fake@example.test";
const FAKE_PASSWORD: &str = "fake-password";
const FAKE_USER_ID: &str = "018f0000-0000-7000-8000-00000000fa4e";
const FAKE_TOKEN: &str = "fake-token-from-the-provider-0123456789";

#[derive(Default)]
struct FakeProvider {
    calls: Mutex<Vec<String>>,
    live_tokens: Mutex<HashSet<String>>,
}

impl FakeProvider {
    fn user() -> AuthUser {
        AuthUser {
            id: FAKE_USER_ID.to_string(),
            email: FAKE_EMAIL.to_string(),
        }
    }

    fn record(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl AuthProvider for FakeProvider {
    fn login<'a>(
        &'a self,
        email: &'a str,
        password: &'a str,
    ) -> BoxFuture<'a, Result<Session, AuthError>> {
        Box::pin(async move {
            self.record(format!("login {email} {password}"));
            if email != FAKE_EMAIL || password != FAKE_PASSWORD {
                return Err(AuthError::InvalidCredentials);
            }
            self.live_tokens
                .lock()
                .unwrap()
                .insert(FAKE_TOKEN.to_string());
            Ok(Session {
                token: FAKE_TOKEN.to_string(),
                expires_at: SystemTime::now() + Duration::from_secs(3600),
                user: Self::user(),
            })
        })
    }

    fn session_user<'a>(
        &'a self,
        token: &'a str,
    ) -> BoxFuture<'a, Result<Option<AuthUser>, AuthError>> {
        Box::pin(async move {
            self.record(format!("session {token}"));
            let live = self.live_tokens.lock().unwrap().contains(token);
            Ok(live.then(Self::user))
        })
    }

    fn logout<'a>(&'a self, token: &'a str) -> BoxFuture<'a, Result<(), AuthError>> {
        Box::pin(async move {
            self.record(format!("logout {token}"));
            self.live_tokens.lock().unwrap().remove(token);
            Ok(())
        })
    }
}

fn serve() -> (TestServer, Arc<FakeProvider>) {
    let provider = Arc::new(FakeProvider::default());
    let server = TestServer::start_with_provider(&offline_config(), provider.clone());
    (server, provider)
}

fn login(server: &TestServer, email: &str, password: &str) -> common::Response {
    server.client.post_json(
        "/api/v1/auth/login",
        &json!({ "email": email, "password": password }),
        &[],
    )
}

#[test]
fn login_is_decided_by_the_provider_and_its_token_becomes_the_session_cookie() {
    let (server, provider) = serve();
    let response = login(&server, FAKE_EMAIL, FAKE_PASSWORD);
    assert_eq!(response.status, 200, "login body: {}", response.text());

    let body = response.json();
    assert_eq!(body["user"]["email"], FAKE_EMAIL, "login body: {body}");
    assert_eq!(body["user"]["id"], FAKE_USER_ID, "login body: {body}");
    assert_eq!(
        response.session_token().as_deref(),
        Some(FAKE_TOKEN),
        "the cookie {SESSION_COOKIE} carries the token that the provider issued"
    );
    assert!(
        provider
            .calls()
            .contains(&format!("login {FAKE_EMAIL} {FAKE_PASSWORD}")),
        "the provider was asked to log in: {:?}",
        provider.calls()
    );
}

#[test]
fn the_provider_resolves_the_session_cookie_to_a_user() {
    let (server, provider) = serve();
    login(&server, FAKE_EMAIL, FAKE_PASSWORD);

    let me = server
        .client
        .get_with("/api/v1/auth/me", &[("Cookie", &cookie(FAKE_TOKEN))]);
    assert_eq!(me.status, 200, "me body: {}", me.text());
    assert_eq!(me.json()["user"]["email"], FAKE_EMAIL);
    assert_eq!(me.json()["user"]["id"], FAKE_USER_ID);
    assert!(
        provider.calls().contains(&format!("session {FAKE_TOKEN}")),
        "the provider was asked about the token: {:?}",
        provider.calls()
    );

    let unknown = server
        .client
        .get_with("/api/v1/auth/me", &[("Cookie", &cookie("not-a-token"))]);
    assert_eq!(unknown.status, 401, "a token the provider does not know");
    assert_eq!(unknown.json()["error"], "unauthorized");

    let anonymous = server.client.get("/api/v1/auth/me");
    assert_eq!(anonymous.status, 401, "no cookie at all");
    assert_eq!(anonymous.json()["error"], "unauthorized");
}

#[test]
fn logout_is_forwarded_to_the_provider_and_the_old_cookie_stops_working() {
    let (server, provider) = serve();
    login(&server, FAKE_EMAIL, FAKE_PASSWORD);

    let response = server.client.request(
        "POST",
        "/api/v1/auth/logout",
        &[("Cookie", &cookie(FAKE_TOKEN))],
        None,
    );
    assert!(
        [200, 204].contains(&response.status),
        "logout status {}: {}",
        response.status,
        response.text()
    );
    assert!(
        provider.calls().contains(&format!("logout {FAKE_TOKEN}")),
        "the provider was asked to end the session: {:?}",
        provider.calls()
    );

    let me = server
        .client
        .get_with("/api/v1/auth/me", &[("Cookie", &cookie(FAKE_TOKEN))]);
    assert_eq!(me.status, 401, "the ended session is no longer accepted");
}
