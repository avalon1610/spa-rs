use std::{future::Future, sync::Arc};

/// Checks a request synchronously
///
///
/// # Example
/// ```
/// # use axum_help::filter::Predicate;
/// # use http::Response;
/// # use http::Request;
/// #
/// struct CheckService;
///
/// impl<ResBody, ReqBody> Predicate<Request<ReqBody>, ResBody> for CheckService
/// where
///     ResBody: Default,
/// {
///     type Request = Request<ReqBody>;
///     type Response = Response<ResBody>;
///
///     fn check(&mut self, mut request: Request<ReqBody>) -> Result<Self::Request, Self::Response> {
///         // do something check
///         Ok(request)   
///     }
/// }
/// ```
pub trait Predicate<Request> {
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
    fn check(&self, request: Request) -> Result<Self::Request, Self::Response>;
}

impl<T, Req, Res, F> Predicate<T> for F
where
    F: Fn(T) -> Result<Req, Res>,
{
    type Request = Req;
    type Response = Res;

    fn check(&self, request: T) -> Result<Self::Request, Self::Response> {
        self(request)
    }
}

/// Checks a request asynchronously
///
/// # Example
/// ```
/// # use axum_help::filter::AsyncPredicate;
/// # use http::Request;
/// # use axum::response::Response;
/// # use std::pin::Pin;
/// # use std::future::Future;
/// #
/// struct CheckService;
///
/// impl<ReqBody, ResBody> AsyncPredicate<Request<ReqBody>, ResBody> for CheckService
/// where
///     ReqBody: Send + 'static,
///     ResBody: Default + Send + 'static,
/// {
///     type Request = Request<ReqBody>;
///     type Response = Response<ResBody>;
///     type Future = Pin<Box<dyn Future<Output = Result<Self::Request, Self::Response>> + Send>>;
///
///     fn check(&mut self, request: Request<ReqBody>) -> Self::Future {
///         Box::pin(async move {
///             // do something check
///             Ok(request)
///         })
///     }
/// }
/// ```
pub trait AsyncPredicate<R> {
    /// The type of requests returned by [`check`](AsyncPredicate::check)
    ///
    /// This request is forwarded to the inner service if the predicate
    /// succeeds.
    type Request;

    /// The type of response return by [`check`](AsyncPredicate::check) if the predicate failed.
    type Response;

    /// The future returned by [`check`](AsyncPredicate::check)
    type Future: Future<Output = Result<Self::Request, Self::Response>>;

    /// Check whether the given request should be forwarded.
    ///
    /// If the future resolves with [`Ok`], the request is forwarded to the inner service.
    fn check(&self, request: R) -> Self::Future;
}

impl<T, Req, Res, U, F> AsyncPredicate<T> for F
where
    F: Fn(T) -> U,
    U: Future<Output = Result<Req, Res>>,
{
    type Request = Req;
    type Response = Res;
    type Future = U;

    fn check(&self, request: T) -> Self::Future {
        self(request)
    }
}

impl<T, R> AsyncPredicate<R> for Arc<T>
where
    T: AsyncPredicate<R>,
{
    type Request = T::Request;
    type Response = T::Response;
    type Future = T::Future;

    fn check(&self, request: R) -> Self::Future {
        (**self).check(request)
    }
}
