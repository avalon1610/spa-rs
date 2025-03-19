//! A tower middleware who can reading and writing session data from Cookie.
//!
use crate::filter::Predicate;
use axum::{extract::Request, http::StatusCode, response::Response};
use headers::{Cookie, HeaderMapExt};
use parking_lot::RwLock;
use std::{cmp::PartialEq, collections::HashMap, sync::Arc};

/// Session object, can access by Extension in RequireSession layer.
///
/// See [RequireSession] example for usage
#[derive(Clone)]
pub struct Session<T> {
    /// current session data
    pub current: T,
    /// session storage
    pub all: Arc<SessionStore<T>>,
}

/// Session storage, can access by Extersion in AddSession layer.
///
/// See [AddSession] example for usage
#[derive(Debug)]
pub struct SessionStore<T> {
    key: String,
    inner: RwLock<HashMap<String, T>>,
}

impl<T: PartialEq> SessionStore<T> {
    /// return new SessionStore with specific key
    pub fn new(key: impl Into<String>) -> Self {
        SessionStore {
            key: key.into(),
            inner: RwLock::new(HashMap::new()),
        }
    }

    /// get the key reference
    pub fn key(&self) -> &str {
        &self.key
    }

    /// insert a new session item
    pub fn insert(&self, k: impl Into<String>, v: T) {
        self.inner.write().insert(k.into(), v);
    }

    /// remove the session item
    pub fn remove(&self, v: T) {
        self.inner.write().retain(|_, x| *x != v);
    }
}

/// Middleware that can access and modify all sessions data. Usually used for **Login** handler
///
/// # Example
///```
/// # use spa_rs::routing::{post, Router};
/// # use spa_rs::Extension;
/// # use spa_rs::session::AddSession;
/// # use spa_rs::session::SessionStore;
/// # use axum_help::filter::FilterExLayer;
/// # use std::sync::Arc;
/// #
/// #[derive(PartialEq, Clone)]
/// struct User;
///
/// async fn login(Extension(session): Extension<Arc<SessionStore<User>>>) {
///     let new_user = User;
///     session.insert("session_id", new_user);
/// }
///
/// #[tokio::main]
/// async fn main() {
///     let session = Arc::new(SessionStore::<User>::new("my_session"));
///     let app = Router::new()
///         .route("/login", post(login))
///         .layer(FilterExLayer::new(AddSession::new(session.clone())));
/// #   axum::Server::bind(&"0.0.0.0:3000".parse().unwrap()).serve(app.into_make_service());
/// }
///```
#[derive(Clone, Debug)]
pub struct AddSession<T>(Arc<SessionStore<T>>);

impl<T> AddSession<T> {
    pub fn new(store: Arc<SessionStore<T>>) -> Self {
        Self(store)
    }
}

impl<T> Predicate<Request> for AddSession<T>
where
    T: Send + Sync + 'static,
{
    type Request = Request;
    type Response = Response;

    fn check(&self, mut request: Request) -> Result<Self::Request, Self::Response> {
        request.extensions_mut().insert(self.0.clone());
        Ok(request)
    }
}

/// Middleware that can access and modify all sessions data.
///
/// # Example
///```
/// # use spa_rs::routing::{post, Router};
/// # use spa_rs::Extension;
/// # use spa_rs::session::RequireSession;
/// # use spa_rs::session::SessionStore;
/// # use spa_rs::session::Session;
/// # use axum_help::filter::FilterExLayer;
/// # use std::sync::Arc;
/// #
/// #[derive(PartialEq, Clone, Debug)]
/// struct User;
///
/// async fn action(Extension(session): Extension<Arc<Session<User>>>) {
///     println!("current user: {:?}", session.current);
/// }
///
/// #[tokio::main]
/// async fn main() {
///     let session = Arc::new(SessionStore::<User>::new("my_session"));
///     let app = Router::new()
///         .route("/action", post(action))
///         .layer(FilterExLayer::new(RequireSession::new(session.clone())));
/// #   axum::Server::bind(&"0.0.0.0:3000".parse().unwrap()).serve(app.into_make_service());
/// }
///```
#[derive(Clone, Debug)]
pub struct RequireSession<T>(Arc<SessionStore<T>>);

impl<T> RequireSession<T> {
    pub fn new(store: Arc<SessionStore<T>>) -> Self {
        Self(store)
    }
}

impl<T> Predicate<Request> for RequireSession<T>
where
    T: Clone + Send + Sync + 'static,
{
    type Request = Request;
    type Response = Response;

    fn check(&self, mut request: Request) -> Result<Self::Request, Self::Response> {
        if let Some(cookie) = request.headers().typed_get::<Cookie>() {
            let sessions = self.0.inner.read();
            for (k, v) in cookie.iter() {
                if k == self.0.key {
                    if let Some(u) = sessions.get(v) {
                        request.extensions_mut().insert(Session {
                            current: u.clone(),
                            all: self.0.clone(),
                        });
                        return Ok(request);
                    }
                }
            }
        }

        Err({
            let mut response = Response::default();
            *response.status_mut() = StatusCode::UNAUTHORIZED;
            response
        })
    }
}
