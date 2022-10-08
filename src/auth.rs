use anyhow::Result;
use async_trait::async_trait;
use axum::{
    body::Bytes,
    headers::{authorization::Basic, Authorization, HeaderMapExt},
    response::{IntoResponse, Response},
    Error,
};
use axum_help::filter::AsyncPredicate;
use http::{Request, StatusCode};
use http_body::combinators::UnsyncBoxBody;
use std::{future::Future, pin::Pin};

#[async_trait]
pub trait AuthCheckPredicate {
    async fn check(
        username: impl Into<String> + Send,
        password: impl Into<String> + Send,
    ) -> Result<()>;
}

#[derive(Clone)]
pub struct AsyncBasicAuth<T>(T)
where
    T: AuthCheckPredicate + Clone;

impl<T> AsyncBasicAuth<T>
where
    T: AuthCheckPredicate + Clone,
{
    pub fn new(p: T) -> Self {
        Self(p)
    }
}

impl<ReqBody, T> AsyncPredicate<Request<ReqBody>, UnsyncBoxBody<Bytes, Error>> for AsyncBasicAuth<T>
where
    T: AuthCheckPredicate + Clone,
    ReqBody: Send + Sync + 'static,
{
    type Request = Request<ReqBody>;
    type Response = Response<UnsyncBoxBody<Bytes, Error>>;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Request, Self::Response>> + Send>>;

    fn check(&mut self, request: Request<ReqBody>) -> Self::Future {
        Box::pin(async move {
            let mut err = "Need basic authenticate".to_string();
            if let Some(authorization) = request.headers().typed_get::<Authorization<Basic>>() {
                if let Err(e) = T::check(authorization.username(), authorization.password()).await {
                    err = format!("check authorization error: {:?}", e);
                } else {
                    return Ok(request);
                }
            }

            Err((
                StatusCode::UNAUTHORIZED,
                [("WWW-Authenticate", "Basic"); 1],
                err,
            )
                .into_response())
        })
    }
}
