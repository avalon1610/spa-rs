use std::future::Future;

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

/// Checks a request asynchronously
pub trait AsyncPredicate<Request, B> {
    /// The type of requests returned by [`check`](AsyncPredicate::check)
    ///
    /// Thies request is forwarded to the inner service if the predicate
    /// succeeds.
    type Request;

    /// The type of response return by [`check`](AsyncPredicate::check) if the predicate failed.
    type Response;

    /// The future returned by [`check`](AsyncPredicate::check)
    type Future: Future<Output = Result<Self::Request, Self::Response>>;

    /// Check whether the given request should be forwarded.
    ///
    /// If the future resolves with [`Ok`], the request is forwarded to the inner service.
    fn check(&mut self, request: Request) -> Self::Future;
}

impl<T, Req, Res, B, U, F> AsyncPredicate<T, B> for F
where
    F: FnMut(T) -> U,
    U: Future<Output = Result<Req, Res>>,
{
    type Request = Req;
    type Response = Res;
    type Future = U;

    fn check(&mut self, request: T) -> Self::Future {
        self(request)
    }
}
