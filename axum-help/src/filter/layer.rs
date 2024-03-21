use super::{AsyncFilterEx, AsyncPredicate, FilterEx};
use std::marker::PhantomData;
use tower::Layer;

/// Conditionally dispatch requests to the inner service based on a synchronous [predicate](super::Predicate).
///
/// This [`Layer`] produces instances of the [`FilterEx`] service.
#[derive(Debug)]
pub struct FilterExLayer<U: Clone> {
    predicate: U,
}

impl<U: Clone> Clone for FilterExLayer<U> {
    fn clone(&self) -> Self {
        Self {
            predicate: self.predicate.clone(),
        }
    }
}

impl<U: Clone> FilterExLayer<U> {
    pub fn new(predicate: U) -> Self {
        Self { predicate }
    }
}

impl<U: Clone, S> Layer<S> for FilterExLayer<U> {
    type Service = FilterEx<S, U>;

    fn layer(&self, inner: S) -> Self::Service {
        FilterEx::new(inner, self.predicate.clone())
    }
}

/// Conditionally dispatch requests to the inner service based on an asynchronous [predicate](super::AsyncPredicate)
///
/// This [`Layer`] produces instances of the [`AsyncFilterEx`] service.
#[derive(Debug)]
pub struct AsyncFilterExLayer<U, R>
where
    U: AsyncPredicate<R>,
{
    predicate: U,
    _r: PhantomData<R>,
}

impl<U: Clone, R> Clone for AsyncFilterExLayer<U, R>
where
    U: AsyncPredicate<R>,
{
    fn clone(&self) -> Self {
        Self {
            predicate: self.predicate.clone(),
            _r: PhantomData,
        }
    }
}

impl<U, R> AsyncFilterExLayer<U, R>
where
    U: AsyncPredicate<R>,
{
    pub fn new(predicate: U) -> Self {
        Self {
            predicate,
            _r: PhantomData,
        }
    }
}

impl<U: Clone, S, R> Layer<S> for AsyncFilterExLayer<U, R>
where
    U: AsyncPredicate<R>,
{
    type Service = AsyncFilterEx<S, U, R>;

    fn layer(&self, inner: S) -> Self::Service {
        AsyncFilterEx::new(inner, self.predicate.clone())
    }
}
