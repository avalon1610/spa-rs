//! Conditionally dispatch requests to the inner service based on the result of
//! a predicate.
//!
//! Unlike [filter](https://docs.rs/tower/latest/tower/filter/index.html) mod in
//! tower, this let you return a custom [`response`](http::response::Response) to user when the request is rejected.
//!
//! # Example
//!```
//! # use axum::routing::{get, Router};
//! # use axum::response::IntoResponse;
//! # use axum::body::Body;
//! # use axum::headers::{authorization::Basic, Authorization, HeaderMapExt};
//! # use axum_help::filter::FilterExLayer;
//! # use http::{Request, StatusCode};
//! #
//! # fn main() {
//!     Router::new()
//!         .route("/get", get(|| async { "get works" }))
//!         .layer(FilterExLayer::new(|request: Request<Body>| {
//!             if let Some(_auth) = request.headers().typed_get::<Authorization<Basic>>() {
//!                 // TODO: do something
//!                 Ok(request)
//!            } else {
//!                Err(StatusCode::UNAUTHORIZED.into_response())
//!            }
//!         }));
//! # }
//!```
//!
use http::{Request, Response};
use pin_project_lite::pin_project;
use std::{
    future::Future,
    marker::PhantomData,
    task::{Context, Poll},
};
use tower::{Layer, Service};

pin_project! {
    /// Filtered response future from [`FilterEx`] services.
    ///
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

/// Checks a request synchronously
pub trait Predicate<Request, B> {
    /// The type of requests returned by [`check`](Predicate::check).
    ///
    /// This request is forwarded to the inner service if the predicate
    /// succeeds.
    type Request;

    /// The type of response return by [`check`](Predicate::check) if the predicate failed.
    type Response;

    /// Check whether the given request should be forwarded.
    ///
    /// If the future resolves with [`Ok`], the request is forwarded to the inner service.
    fn check(&mut self, request: Request) -> Result<Self::Request, Self::Response>;
}

impl<T, Req, Res, B, F> Predicate<T, B> for F
where
    F: FnMut(T) -> Result<Req, Res>,
{
    type Request = Req;
    type Response = Res;

    fn check(&mut self, request: T) -> Result<Self::Request, Self::Response> {
        self(request)
    }
}

/// Conditionally dispatch requests to the inner service based on a [predicate].
///
/// [predicate]: Predicate
#[derive(Debug)]
pub struct FilterEx<T, U, B> {
    inner: T,
    predicate: U,
    p: PhantomData<B>,
}

impl<T: Clone, U: Clone, B> Clone for FilterEx<T, U, B> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            predicate: self.predicate.clone(),
            p: PhantomData,
        }
    }
}

impl<T, U, B> FilterEx<T, U, B> {
    pub fn new(inner: T, predicate: U) -> Self {
        Self {
            inner,
            predicate,
            p: PhantomData,
        }
    }
}

impl<T, U, ReqBody, ResBody> Service<Request<ReqBody>> for FilterEx<T, U, ResBody>
where
    T: Service<U::Request, Response = Response<ResBody>>,
    U: Predicate<Request<ReqBody>, ResBody, Response = Response<ResBody>>,
{
    type Response = T::Response;
    type Error = T::Error;
    type Future = ResponseFuture<T::Future, ResBody>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        match self.predicate.check(req) {
            Ok(req) => ResponseFuture::Future {
                future: self.inner.call(req),
            },
            Err(response) => ResponseFuture::Error {
                response: Some(response),
            },
        }
    }
}

/// Conditionally dispatch requests to the inner service based on a synchronous [predicate](Predicate).
///
/// This [`Layer`] produces instances of the [`FilterEx`] service.
#[derive(Debug, Clone)]
pub struct FilterExLayer<U, B> {
    predicate: U,
    p: PhantomData<B>,
}

impl<U, B> FilterExLayer<U, B> {
    pub fn new(predicate: U) -> Self {
        Self {
            predicate,
            p: PhantomData,
        }
    }
}

impl<U: Clone, S, B> Layer<S> for FilterExLayer<U, B> {
    type Service = FilterEx<S, U, B>;

    fn layer(&self, inner: S) -> Self::Service {
        FilterEx::new(inner, self.predicate.clone())
    }
}
