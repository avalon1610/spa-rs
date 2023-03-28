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
    type CheckInfo: Send + Sync + 'static;

    async fn check(
        username: impl Into<String> + Send,
        password: impl Into<String> + Send,
    ) -> Result<Self::CheckInfo>;
}

#[derive(Clone)]
pub struct AsyncBasicAuth<T>(T, String)
where
    T: AuthCheckPredicate + Clone;

impl<T> AsyncBasicAuth<T>
where
    T: AuthCheckPredicate + Clone,
{
    pub fn new(p: T) -> Self {
        Self(p, "Need basic authenticate".to_string())
    }

    pub fn err_msg(mut self, msg: impl Into<String>) -> Self {
        self.1 = msg.into();
        self
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

    fn check(&mut self, mut request: Request<ReqBody>) -> Self::Future {
        let mut err = self.1.clone();
        Box::pin(async move {
            if let Some(authorization) = request.headers().typed_get::<Authorization<Basic>>() {
                match T::check(authorization.username(), authorization.password()).await {
                    Err(e) => err = format!("check authorization error: {:?}", e),
                    Ok(ci) => {
                        request.extensions_mut().insert(ci);
                        return Ok(request);
                    }
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
