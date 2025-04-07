use std::{future::Future, sync::Arc};

/// Checks a request synchronously
///
///
/// # Example
/// ```
/// # use axum_help::filter::Predicate;
/// # use axum::response::Response;
/// # use axum::extract::Request;
///
/// struct CheckService;
///
/// impl Predicate<Request> for CheckService
/// {
///     type Request = Request;
///     type Response = Response;
///
///     fn check(&self, mut request: Request) -> Result<Self::Request, Self::Response> {
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
    /// If the check returns with [`Ok`], the request is forwarded to the inner service.
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
/// ```rust
/// # use axum_help::filter::AsyncPredicate;
/// # use axum::extract::Request;
/// # use axum::response::Response;
/// # use std::pin::Pin;
/// # use std::future::Future;
/// #
/// struct CheckService;
///
/// impl AsyncPredicate<Request> for CheckService {
///     type Response = Response;
///     type Future = Pin<Box<dyn Future<Output = Result<Request, Self::Response>> + Send>>;
///
///     fn check(&self, request: Request) -> Self::Future {
///         Box::pin(async move {
///             // do something check
///             Ok(request)
///         })
///     }
/// }
/// ```
pub trait AsyncPredicate<Req> {
    /// The type of response return by [`check`](AsyncPredicate::check) if the predicate failed.
    type Response;

    /// The future returned by [`check`](AsyncPredicate::check)
    type Future: Future<Output = Result<Req, Self::Response>>;

    /// Check whether the given request should be forwarded.
    ///
    /// If the future resolves with [`Ok`], the request is forwarded to the inner service.
    fn check(&self, request: Req) -> Self::Future;
}

impl<Req, Res, U, F> AsyncPredicate<Req> for F
where
    F: Fn(Req) -> U,
    U: Future<Output = Result<Req, Res>>,
{
    type Response = Res;
    type Future = U;

    fn check(&self, request: Req) -> Self::Future {
        self(request)
    }
}

impl<T, R> AsyncPredicate<R> for Arc<T>
where
    T: AsyncPredicate<R>,
{
    type Response = T::Response;
    type Future = T::Future;

    fn check(&self, request: R) -> Self::Future {
        (**self).check(request)
    }
}
