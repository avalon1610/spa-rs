use axum::headers::{Cookie, HeaderMapExt};
use http::{Request, Response, StatusCode};
use http_body::Body;
use parking_lot::RwLock;
use pin_project_lite::pin_project;
use std::{
    collections::HashMap,
    future::Future,
    sync::Arc,
    task::{Context, Poll},
};
use tower::{Layer, Service};

#[derive(Clone)]
pub struct RequireSession<S, T> {
    inner: S,
    store: Arc<SessionStore<T>>,
}

impl<S, T> RequireSession<S, T> {
    pub fn new(inner: S, store: Arc<SessionStore<T>>) -> Self {
        RequireSession { inner, store }
    }
}

pub struct SessionStore<T> {
    key: String,
    inner: RwLock<HashMap<String, T>>,
}

impl<T> SessionStore<T> {
    pub fn new(key: impl Into<String>) -> Self {
        SessionStore {
            key: key.into(),
            inner: RwLock::new(HashMap::new()),
        }
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn data(&self) -> &RwLock<HashMap<String, T>> {
        &self.inner
    }
}

pub struct AddSessionLayer<T>(Arc<SessionStore<T>>);

impl<T> AddSessionLayer<T> {
    pub fn new(store: Arc<SessionStore<T>>) -> Self {
        Self(store)
    }
}

#[derive(Clone)]
pub struct AddSession<S, T> {
    inner: S,
    store: Arc<SessionStore<T>>,
}

impl<S, T> Layer<S> for AddSessionLayer<T> {
    type Service = AddSession<S, T>;

    fn layer(&self, inner: S) -> Self::Service {
        AddSession {
            inner,
            store: self.0.clone(),
        }
    }
}

impl<S, T, ReqBody, ResBody> Service<Request<ReqBody>> for AddSession<S, T>
where
    T: Send + Sync + 'static,
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<ReqBody>) -> Self::Future {
        req.extensions_mut().insert(self.store.clone());
        self.inner.call(req)
    }
}

pub struct RequireSessionLayer<T>(Arc<SessionStore<T>>);

impl<T> RequireSessionLayer<T> {
    pub fn new(store: Arc<SessionStore<T>>) -> Self {
        Self(store)
    }
}

impl<S, T> Layer<S> for RequireSessionLayer<T> {
    type Service = RequireSession<S, T>;

    fn layer(&self, inner: S) -> Self::Service {
        RequireSession::new(inner, self.0.clone())
    }
}

pin_project! {
    #[project = ResponseKind]
    pub enum ResponseFuture<F, B> {
        Future {#[pin] future: F },
        Error { response: Option<Response<B>> },
    }
}

impl<F, B, E> Future for ResponseFuture<F, B>
where
    F: Future<Output = Result<Response<B>, E>>,
{
    type Output = F::Output;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.project() {
            ResponseKind::Future { future } => future.poll(cx),
            ResponseKind::Error { response } => {
                let response = response.take().unwrap();
                Poll::Ready(Ok(response))
            }
        }
    }
}

impl<S, ReqBody, ResBody, T> Service<Request<ReqBody>> for RequireSession<S, T>
where
    T: Clone + Send + Sync + 'static,
    ResBody: Default + Body,
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future, ResBody>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<ReqBody>) -> Self::Future {
        if let Some(cookie) = req.headers().typed_get::<Cookie>() {
            let sessions = self.store.inner.read();
            for (k, v) in cookie.iter() {
                if k == self.store.key {
                    if let Some(u) = sessions.get(v) {
                        req.extensions_mut().insert((u.clone(), self.store.clone()));
                        return ResponseFuture::Future {
                            future: self.inner.call(req),
                        };
                    }
                }
            }
        }

        ResponseFuture::Error {
            response: Some({
                let mut response = Response::new(ResBody::default());
                *response.status_mut() = StatusCode::UNAUTHORIZED;
                response
            }),
        }
    }
}
