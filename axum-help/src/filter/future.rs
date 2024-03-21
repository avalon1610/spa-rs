use super::predicate::AsyncPredicate;
use axum::response::Response;
use futures_core::ready;
use pin_project_lite::pin_project;
use std::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};
use tower::Service;

pin_project! {
    /// Filtered response future from [`FilterEx`] services.
    ///
    #[project = ResponseKind]
    pub enum ResponseFuture<F> {
        Future {#[pin] future: F },
        Error { response: Option<Response> },
    }
}

impl<F, E> Future for ResponseFuture<F>
where
    F: Future<Output = Result<Response, E>>,
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.project() {
            ResponseKind::Future { future } => future.poll(cx),
            ResponseKind::Error { response } => {
                let response = response.take().unwrap();
                Poll::Ready(Ok(response))
            }
        }
    }
}

pin_project! {
    /// Filtered response future from [`AsyncFilterEx`](super::AsyncFilterEx) services.
    ///
    pub struct AsyncResponseFuture<P, S, R>
    where
        P:  AsyncPredicate<R>,
        S: Service<P::Request>,
    {
        #[pin]
        state: State<P::Future, S::Future>,
        service: S,
        _p: PhantomData<P>
    }
}

pin_project! {
    #[project = StateProj]
    #[derive(Debug)]
    enum State<F, G> {
        /// Waiting for the predicate future
        Check { #[pin] check: F},
        /// Waiting for the response future
        WaitResponse { #[pin] response: G}
    }
}

impl<P, S, R> AsyncResponseFuture<P, S, R>
where
    P: AsyncPredicate<R>,
    S: Service<P::Request>,
{
    pub(super) fn new(check: P::Future, service: S) -> Self {
        Self {
            state: State::Check { check },
            service,
            _p: PhantomData,
        }
    }
}

impl<P, S, R> Future for AsyncResponseFuture<P, S, R>
where
    P: AsyncPredicate<R>,
    S: Service<P::Request, Response = <P as AsyncPredicate<R>>::Response>,
{
    type Output = Result<S::Response, S::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        loop {
            match this.state.as_mut().project() {
                StateProj::Check { mut check } => match ready!(check.as_mut().poll(cx)) {
                    Ok(request) => {
                        let response = this.service.call(request);
                        this.state.set(State::WaitResponse { response });
                    }
                    Err(e) => {
                        return Poll::Ready(Ok(e));
                    }
                },

                StateProj::WaitResponse { response } => {
                    return response.poll(cx);
                }
            }
        }
    }
}
