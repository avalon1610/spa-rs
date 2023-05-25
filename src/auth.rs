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
use parking_lot::Mutex;
use std::{
    collections::VecDeque,
    fmt::{Debug, Display},
    future::Future,
    pin::Pin,
    sync::Arc,
};

use self::digest::unauthorized;

#[async_trait]
pub trait AuthCheckPredicate {
    type CheckInfo: Send + Sync + 'static;

    async fn check(
        &self,
        username: impl Into<String> + Send,
        password: impl Into<String> + Send,
    ) -> Result<Self::CheckInfo>;

    fn username(&self) -> &str;
    fn password(&self) -> &str;
}

#[derive(Clone)]
pub struct AsyncBasicAuth<T>(T, String)
where
    T: AuthCheckPredicate + Clone + Send;

impl<T> AsyncBasicAuth<T>
where
    T: AuthCheckPredicate + Clone + Send,
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
    T: AuthCheckPredicate + Clone + Send + Sync + 'static,
    ReqBody: Send + Sync + 'static,
{
    type Request = Request<ReqBody>;
    type Response = Response<UnsyncBoxBody<Bytes, Error>>;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Request, Self::Response>> + Send>>;

    fn check(&mut self, mut request: Request<ReqBody>) -> Self::Future {
        let mut err = self.1.clone();
        let auth = self.0.clone();
        Box::pin(async move {
            if let Some(authorization) = request.headers().typed_get::<Authorization<Basic>>() {
                match auth
                    .check(authorization.username(), authorization.password())
                    .await
                {
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

#[derive(Clone)]
pub struct AsyncDigestAuth<T>
where
    T: AuthCheckPredicate + Clone + Send,
{
    inner: T,
    err: String,
    nonces: Arc<Mutex<VecDeque<(String, String)>>>,
}

impl<T> AsyncDigestAuth<T>
where
    T: AuthCheckPredicate + Clone + Send,
{
    pub fn new(p: T) -> Self {
        Self {
            inner: p,
            err: "Need digest authenticate".to_string(),
            nonces: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub fn err_msg(mut self, msg: impl Into<String>) -> Self {
        self.err = msg.into();
        self
    }
}

impl<ReqBody, T> AsyncPredicate<Request<ReqBody>, UnsyncBoxBody<Bytes, Error>>
    for AsyncDigestAuth<T>
where
    T: AuthCheckPredicate + Clone + Send + Sync + 'static,
    ReqBody: Send + Sync + 'static + Debug,
{
    type Request = Request<ReqBody>;
    type Response = Response<UnsyncBoxBody<Bytes, Error>>;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Request, Self::Response>> + Send>>;

    fn check(&mut self, request: Request<ReqBody>) -> Self::Future {
        let err = self.err.clone();
        let inner = self.inner.clone();
        let nonces = self.nonces.clone();
        Box::pin(async move {
            if let Some(auth_header) = request.headers().get("Authorization") {
                let auth = digest::Authorization::from_header(
                    auth_header.to_str().map_err(|e| bad_request(e))?,
                )
                .map_err(|e| bad_request(e))?;

                return auth.check(inner.username(), inner.password(), nonces, request);
            }

            Err(unauthorized(nonces, err))
        })
    }
}

fn bad_request(e: impl Display) -> Response {
    (
        StatusCode::BAD_REQUEST,
        format!("Bad request in header Authorization: {}", e),
    )
        .into_response()
}

mod digest {
    use anyhow::{anyhow, bail, Result};
    use axum::response::{IntoResponse, Response};
    use http::{Request, StatusCode};
    use parking_lot::Mutex;
    use rand::{distributions::Alphanumeric, thread_rng, Rng};
    use std::{collections::VecDeque, fmt::Debug, sync::Arc};

    #[derive(Default, Debug)]
    pub(super) struct Authorization {
        pub(super) username: String,
        pub(super) realm: String,
        pub(super) nonce: String,
        pub(super) uri: String,
        pub(super) qop: String,
        pub(super) nc: String,
        pub(super) cnonce: String,
        pub(super) response: String,
        pub(super) opaque: String,
    }

    impl Authorization {
        pub(super) fn check<B1>(
            &self,
            username: impl AsRef<str>,
            password: impl AsRef<str>,
            nonces: Arc<Mutex<VecDeque<(String, String)>>>,
            request: Request<B1>,
        ) -> Result<Request<B1>, Response>
        where
            B1: Debug,
        {
            let mut found_nonce = false;
            {
                let mut nonce_list = nonces.lock();
                let mut index = nonce_list.len().saturating_sub(1);

                for (nonce, opaque) in nonce_list.iter().rev() {
                    if nonce == &self.nonce || opaque == &self.opaque {
                        found_nonce = true;
                        nonce_list.remove(index);
                        break;
                    }

                    index = index.saturating_sub(1);
                }
            }

            if !found_nonce {
                return Err(unauthorized(nonces, "invalid nonce or opaque"));
            }

            log::debug!("digest request: {:?}", request);
            let ha1 = md5::compute(format!(
                "{}:{}:{}",
                username.as_ref(),
                self.realm,
                password.as_ref()
            ));
            let ha2 = md5::compute(format!("{}:{}", request.method().to_string(), self.uri));
            let password = md5::compute(format!(
                "{:x}:{}:{}:{}:{}:{:x}",
                ha1, self.nonce, self.nc, self.cnonce, self.qop, ha2
            ));

            if format!("{:x}", password) != self.response {
                return Err(unauthorized(nonces, "invalid username or password"));
            }

            Ok(request)
        }

        const DIGEST_MARK: &str = "Digest";
        pub(super) fn from_header(auth: impl AsRef<str>) -> Result<Self> {
            let auth = auth.as_ref();
            let (mark, content) = auth.split_at(Self::DIGEST_MARK.len());
            let content = content.trim();
            if mark != Self::DIGEST_MARK {
                bail!("only support digest authorization");
            }

            let mut result = Authorization::default();
            for c in content.split(',').into_iter() {
                let c = c.trim();
                let (k, v) = c
                    .split_once('=')
                    .ok_or_else(|| anyhow!("invalid part of authorization: {}", c))?;
                let v = v.trim_matches('"');
                match k {
                    "username" => result.username = v.to_string(),
                    "realm" => result.realm = v.to_string(),
                    "nonce" => result.nonce = v.to_string(),
                    "uri" => result.uri = v.to_string(),
                    "qop" => result.qop = v.to_string(),
                    "nc" => result.nc = v.to_string(),
                    "cnonce" => result.cnonce = v.to_string(),
                    "response" => result.response = v.to_string(),
                    "opaque" => result.opaque = v.to_string(),
                    _ => {
                        log::warn!("unknown authorization part: {}", c);
                        continue;
                    }
                }
            }

            log::debug!("digest auth: {:?}", result);
            Ok(result)
        }
    }

    pub(super) fn unauthorized(
        nonces: Arc<Mutex<VecDeque<(String, String)>>>,
        msg: impl Into<String>,
    ) -> Response {
        let realm = format!("Login to {}", env!("CARGO_PKG_NAME"));
        let nonce = rand_string(32);
        let opaque = rand_string(32);

        let www_authenticate = format!(
            r#"Digest realm="{}",qop="auth",nonce="{}",opaque="{}""#,
            realm, nonce, opaque
        );

        {
            let mut nonce_list = nonces.lock();
            while nonce_list.len() >= 256 {
                nonce_list.pop_front();
            }

            nonce_list.push_back((nonce, opaque));
        }

        (
            StatusCode::UNAUTHORIZED,
            [("WWW-Authenticate", www_authenticate); 1],
            msg.into(),
        )
            .into_response()
    }

    fn rand_string(count: usize) -> String {
        thread_rng()
            .sample_iter(Alphanumeric)
            .take(count)
            .map(char::from)
            .collect()
    }
}
