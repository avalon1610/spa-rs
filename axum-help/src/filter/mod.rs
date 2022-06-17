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
use future::{AsyncResponseFuture, ResponseFuture};
use http::{Request, Response};
pub use layer::{AsyncFilterExLayer, FilterExLayer};
pub use predicate::{AsyncPredicate, Predicate};
use std::{
    marker::PhantomData,
    task::{Context, Poll},
};
use tower::Service;

mod future;
mod layer;
mod predicate;

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
    /// Returns a new [FilterEx] service wrapping `inner`
    pub fn new(inner: T, predicate: U) -> Self {
        Self {
            inner,
            predicate,
            p: PhantomData,
        }
    }

    /// Returns a new [Layer](tower::Layer) that wraps services with a [FilterEx] service
    /// with the given [Predicate]
    ///
    pub fn layer(predicate: U) -> FilterExLayer<U, B> {
        FilterExLayer::new(predicate)
    }

    /// Check a `Request` value against thie filter's predicate
    pub fn check<R>(&mut self, request: R) -> Result<U::Request, U::Response>
    where
        U: Predicate<R, B>,
    {
        self.predicate.check(request)
    }

    /// Get a reference to the inner service
    pub fn get_ref(&self) -> &T {
        &self.inner
    }

    /// Get a mutable reference to the inner service
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    /// Consume `self`, returning the inner service
    pub fn into_inner(self) -> T {
        self.inner
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

/// Conditionally dispatch requests to the inner service based on an
/// asynchronous [predicate](AsyncPredicate)
///
#[derive(Debug)]
pub struct AsyncFilterEx<T, U, B> {
    inner: T,
    predicate: U,
    p: PhantomData<B>,
}

impl<T: Clone, U: Clone, B> Clone for AsyncFilterEx<T, U, B> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            predicate: self.predicate.clone(),
            p: PhantomData,
        }
    }
}

impl<T, U, B> AsyncFilterEx<T, U, B> {
    /// Returns a new [AsyncFilterEx] service wrapping `inner`.
    pub fn new(inner: T, predicate: U) -> Self {
        Self {
            inner,
            predicate,
            p: PhantomData,
        }
    }

    /// Returns a new [Layer](tower::Layer) that wraps services with a [AsyncFilterEx] service
    /// with the given [AsyncPredicate]
    ///
    pub fn layer(predicate: U) -> AsyncFilterExLayer<U, B> {
        AsyncFilterExLayer::new(predicate)
    }

    /// Check a `Request` value against thie filter's predicate
    pub async fn check<R>(&mut self, request: R) -> Result<U::Request, U::Response>
    where
        U: AsyncPredicate<R, B>,
    {
        self.predicate.check(request).await
    }

    /// Get a reference to the inner service
    pub fn get_ref(&self) -> &T {
        &self.inner
    }

    /// Get a mutable reference to the inner service
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    /// Consume `self`, returning the inner service
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T, U, ReqBody, ResBody> Service<Request<ReqBody>> for AsyncFilterEx<T, U, ResBody>
where
    T: Service<U::Request, Response = Response<ResBody>> + Clone,
    U: AsyncPredicate<Request<ReqBody>, ResBody, Response = Response<ResBody>>,
{
    type Response = T::Response;
    type Error = T::Error;
    type Future = AsyncResponseFuture<U, T, Request<ReqBody>, ResBody>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        use std::mem;

        let inner = self.inner.clone();
        // In case the inner service has state that's driven to readiness and
        // not tracked by clones (such as `Buffer`), pass the version we have
        // already called `poll_ready` on into the future, and leave its clone
        // behind.
        let inner = mem::replace(&mut self.inner, inner);

        // Check the request
        let check = self.predicate.check(req);

        AsyncResponseFuture::new(check, inner)
    }
}
