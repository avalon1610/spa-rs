use super::{AsyncFilterEx, FilterEx};
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
pub struct AsyncFilterExLayer<U> {
    predicate: U,
}

impl<U: Clone> Clone for AsyncFilterExLayer<U> {
    fn clone(&self) -> Self {
        Self {
            predicate: self.predicate.clone(),
        }
    }
}

impl<U> AsyncFilterExLayer<U> {
    pub fn new(predicate: U) -> Self {
        Self { predicate }
    }
}

impl<U: Clone, S> Layer<S> for AsyncFilterExLayer<U> {
    type Service = AsyncFilterEx<S, U>;

    fn layer(&self, inner: S) -> Self::Service {
        AsyncFilterEx::new(inner, self.predicate.clone())
    }
}
