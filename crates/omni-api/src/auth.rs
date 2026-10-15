//! Authentication. Everything that decides who a request comes from goes
//! through the `AuthProvider` trait, so an external identity provider can
//! replace the built-in password login without any other module changing.

use std::future::Future;
use std::pin::Pin;
use std::time::SystemTime;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The user a session belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthUser {
    /// The user's UUID.
    pub id: String,
    pub email: String,
}

/// A login that succeeded: the opaque token that goes into the session cookie.
#[derive(Debug, Clone)]
pub struct Session {
    pub token: String,
    pub expires_at: SystemTime,
    pub user: AuthUser,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// Unknown user or wrong password; the two are never told apart.
    InvalidCredentials,
    /// The provider cannot answer right now.
    Unavailable(String),
}

pub trait AuthProvider: Send + Sync + 'static {
    /// Checks an email and password and starts a session.
    fn login<'a>(
        &'a self,
        email: &'a str,
        password: &'a str,
    ) -> BoxFuture<'a, Result<Session, AuthError>>;

    /// The user of a session token, or `None` when it is unknown, expired or ended.
    fn session_user<'a>(
        &'a self,
        token: &'a str,
    ) -> BoxFuture<'a, Result<Option<AuthUser>, AuthError>>;

    /// Ends the session of a token.
    fn logout<'a>(&'a self, token: &'a str) -> BoxFuture<'a, Result<(), AuthError>>;
}
