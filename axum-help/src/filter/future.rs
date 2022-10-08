use super::predicate::AsyncPredicate;
use futures_core::ready;
use http::Response;
use pin_project_lite::pin_project;
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};
use tower::Service;

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
    pub struct AsyncResponseFuture<P, S, R, B>
    where
        P: AsyncPredicate<R, B>,
        S: Service<P::Request>,
    {
        #[pin]
        state: State<P::Future, S::Future>,
        service: S
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

impl<P, S, R, B> AsyncResponseFuture<P, S, R, B>
where
    P: AsyncPredicate<R, B>,
    S: Service<P::Request>,
{
    pub(super) fn new(check: P::Future, service: S) -> Self {
        Self {
            state: State::Check { check },
            service,
        }
    }
}

impl<P, S, R, B> Future for AsyncResponseFuture<P, S, R, B>
where
    P: AsyncPredicate<R, B>,
    S: Service<P::Request, Response = <P as AsyncPredicate<R, B>>::Response>,
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
