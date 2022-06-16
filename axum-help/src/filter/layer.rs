use super::{AsyncFilterEx, FilterEx};
use std::marker::PhantomData;
use tower::Layer;

/// Conditionally dispatch requests to the inner service based on a synchronous [predicate](super::Predicate).
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

/// Conditionally dispatch requests to the inner service based on an asynchronous [predicate](super::AsyncPredicate)
/// 
/// This [`Layer`] produces instances of the [`AsyncFilterEx`] service.
#[derive(Debug)]
pub struct AsyncFilterExLayer<U, B> {
    predicate: U,
    p: PhantomData<B>,
}

impl<U, B> AsyncFilterExLayer<U, B> {
    pub fn new(predicate: U) -> Self {
        Self {
            predicate,
            p: PhantomData,
        }
    }
}

impl<U: Clone, S, B> Layer<S> for AsyncFilterExLayer<U, B> {
    type Service = AsyncFilterEx<S, U, B>;

    fn layer(&self, inner: S) -> Self::Service {
        AsyncFilterEx::new(inner, self.predicate.clone())
    }
}
