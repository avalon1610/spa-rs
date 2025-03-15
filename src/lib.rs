//! spa-rs is a library who can embed all SPA web application files (dist static file),
//! and release as a single binary executable.
//!
//! It based-on [axum] and [rust_embed]
//!
//! It reexported all axum module for convenient use.
//! # Example
//! ```no_run
//! use spa_rs::spa_server_root;
//! use spa_rs::SpaServer;
//! use spa_rs::routing::{get, Router};
//! use anyhow::Result;
//!
//! spa_server_root!("web/dist");           // specific your SPA dist file location
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     let data = String::new();           // server context can be acccess by [axum::Extension]
//!     let mut srv = SpaServer::new()?
//!         .port(3000)
//!         .data(data)
//!         .static_path("/png", "web")     // static file generated in runtime
//!         .route("/api", Router::new()
//!             .route("/get", get(|| async { "get works" })
//!         )
//!     );
//!     srv.run(spa_server_root!()).await?;  
//!
//!     Ok(())
//! }
//! ```
//!
//! # Session
//! See [session] module for more detail.
//!
//! # Dev
//! When writing SPA application, you may want use hot-reload functionallity provided
//! by SPA framework. such as [`vite dev`] or [`ng serve`].
//!
//! You can use spa-rs to reverse proxy all static requests to SPA framework. (need enable `reverse-proxy` feature)
//!
//! ## Example
//! ```ignore
//!   let forward_addr = "http://localhost:1234";
//!   srv.reverse_proxy(forward_addr.parse()?);
//! ```
use anyhow::{anyhow, Context, Result};
use axum::{
    body::{Bytes, HttpBody},
    extract::Request,
    http::HeaderValue,
    response::Response,
    routing::{get_service, Route},
};
#[cfg(feature = "openssl")]
use axum_server::tls_openssl::OpenSSLConfig;
#[cfg(feature = "rustls")]
use axum_server::tls_rustls::RustlsConfig;
use futures_util::future::BoxFuture;
use http::{
    header::{self},
    StatusCode,
};
#[cfg(feature = "reverse-proxy")]
use http::{Method, Uri};
use log::{debug, error, warn};
pub use rust_embed;
use std::{
    collections::HashMap,
    convert::Infallible,
    env::current_exe,
    fs::{self, create_dir_all},
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
};
use tower::{util::ServiceExt as TowerServiceExt, Layer, Service};
use tower_http::{
    services::{ServeDir, ServeFile},
    set_header::SetResponseHeaderLayer,
};

pub use axum::*;
pub mod auth;
pub mod session;
pub use axum::debug_handler;
pub use axum_help::*;

/// A server wrapped axum server.
///
/// It can:
/// - serve static files in SPA root path
/// - serve API requests in router
/// - fallback to SPA static file when route matching failed
///     - if still get 404, it will redirect to SPA index.html
///
#[derive(Default)]
pub struct SpaServer<T = ()>
where
    T: Clone + Send + Sync + 'static,
{
    static_path: Vec<(String, PathBuf)>,
    port: u16,
    router: Router,
    data: Option<T>,
    forward: Option<String>,
    release_path: PathBuf,
    extra_layer: Vec<Box<dyn FnOnce(Router) -> Router>>,
    host_routers: HashMap<String, Router>,
}

#[axum::debug_handler]
#[cfg(feature = "reverse-proxy")]
async fn forwarded_to_dev(
    Extension(forward_addr): Extension<String>,
    uri: Uri,
    method: Method,
) -> HttpResult<Response> {
    use axum::http::Response;
    use http::uri::Scheme;

    if method == Method::GET {
        let client = reqwest::Client::builder().no_proxy().build()?;
        let mut parts = uri.into_parts();
        parts.authority = Some(forward_addr.parse()?);
        if parts.scheme.is_none() {
            parts.scheme = Some(Scheme::HTTP);
        }
        let url = Uri::from_parts(parts)?.to_string();

        println!("forward url: {}", url);
        let response = client.get(url).send().await?;
        let response: http::Response<_> = response.into();
        let (parts, body) = response.into_parts();
        let body = body.as_bytes().map(|b| b.to_vec()).unwrap_or_default();

        let response = Response::from_parts(parts, body.into());
        return Ok(response);
    }

    Err(HttpError {
        message: "Method not allowed".to_string(),
        status_code: StatusCode::METHOD_NOT_ALLOWED,
    })
}

#[cfg(not(feature = "reverse-proxy"))]
async fn forwarded_to_dev() {
    unreachable!("reverse-proxy not enabled, should never call forwarded_to_dev")
}

impl<T> SpaServer<T>
where
    T: Clone + Send + Sync + 'static,
{
    /// Just new(), nothing special
    pub fn new() -> Result<Self> {
        Ok(Self {
            static_path: Vec::new(),
            port: 8080,
            forward: None,
            release_path: current_exe()?
                .parent()
                .ok_or_else(|| anyhow!("no parent in current_exe"))?
                .join(format!(".{}_static_files", env!("CARGO_PKG_NAME"))),
            extra_layer: Vec::new(),
            host_routers: HashMap::new(),
            router: Router::new(),
            data: None,
        })
    }

    /// Specific server context data
    ///
    /// This is similar to [axum middleware](https://docs.rs/axum/latest/axum/#middleware)
    pub fn data(mut self, data: T) -> Self
    where
        T: Clone + Send + Sync + 'static,
    {
        self.data = Some(data);
        self
    }

    /// Specific an axum layer to server
    ///
    /// This is similar to [axum middleware](https://docs.rs/axum/latest/axum/#middleware)
    pub fn layer<L, NewResBody>(mut self, layer: L) -> Self
    where
        L: Layer<Route> + Clone + Send + 'static,
        L::Service: Service<Request, Response = Response<NewResBody>, Error = Infallible>
            + Clone
            + Send
            + 'static,
        <L::Service as Service<Request>>::Future: Send + 'static,
        NewResBody: HttpBody<Data = Bytes> + Send + 'static,
        NewResBody::Error: Into<BoxError>,
    {
        self.extra_layer.push(Box::new(move |app| app.layer(layer)));
        self
    }

    /// make a reverse proxy which redirect all SPA requests to dev server, such as `ng serve`, `vite`.  
    ///
    /// it's useful when debugging UI
    #[cfg(feature = "reverse-proxy")]
    #[cfg_attr(docsrs, doc(cfg(feature = "reverse-proxy")))]
    pub fn reverse_proxy(mut self, addr: impl Into<String>) -> Self {
        self.forward = Some(addr.into());
        self
    }

    /// static file release path in runtime
    ///
    /// Default path is /tmp/[env!(CARGO_PKG_NAME)]_static_files
    pub fn release_path(mut self, rp: impl Into<PathBuf>) -> Self {
        self.release_path = rp.into();
        self
    }

    /// Run the spa server forever
    pub async fn run<Root>(self, root: Root) -> Result<()>
    where
        Root: SpaStatic,
    {
        self.run_raw(Some(root), None).await
    }

    /// Run the spa server with tls
    #[cfg(any(feature = "openssl", feature = "rustls"))]
    pub async fn run_tls<Root>(self, root: Root, config: HttpsConfig) -> Result<()>
    where
        Root: SpaStatic,
    {
        self.run_raw(Some(root), Some(config)).await
    }

    /// Run the spa server without spa root
    pub async fn run_api(self) -> Result<()> {
        self.run_raw::<ApiOnly>(None, None).await
    }

    /// Run the spa server with tls and without spa root
    #[cfg(any(feature = "openssl", feature = "rustls"))]
    pub async fn run_api_tls(self, config: HttpsConfig) -> Result<()> {
        self.run_raw::<ApiOnly>(None, Some(config)).await
    }

    /// Run the spa server with or without spa root, and with or without tls
    async fn run_raw<Root>(mut self, root: Option<Root>, config: Option<HttpsConfig>) -> Result<()>
    where
        Root: SpaStatic,
    {
        if let Some(root) = root {
            let embeded_dir = root.release(self.release_path)?;
            let index_file = embeded_dir.clone().join("index.html");

            self.router = if let Some(addr) = self.forward {
                self.router
                    .fallback(forwarded_to_dev)
                    .layer(Extension(addr))
            } else {
                self.router.fallback_service(
                    get_service(ServeDir::new(&embeded_dir).fallback(ServeFile::new(index_file)))
                        .layer(Self::add_cache_control())
                        .handle_error(|e: anyhow::Error| async move {
                            (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                format!(
                            "Unhandled internal server error {:?} when serve embeded path {}",
                            e,
                            embeded_dir.display()
                        ),
                            )
                        }),
                )
            };
        }

        for sf in self.static_path {
            self.router = self.router.nest_service(
                &sf.0,
                get_service(ServeDir::new(&sf.1))
                    .layer(Self::add_cache_control())
                    .handle_error(|e: anyhow::Error| async move {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!(
                                "Unhandled internal server error {:?} when serve static path {}",
                                e,
                                sf.1.display()
                            ),
                        )
                    }),
            )
        }

        self.router = self
            .router
            .layer(MatchHostLayer::new(Arc::new(self.host_routers.clone())));

        if let Some(data) = self.data {
            self.router = self.router.layer(Extension(data));
        }

        for layer in self.extra_layer {
            self.router = layer(self.router)
        }

        let addr = format!("0.0.0.0:{}", self.port).parse()?;
        #[allow(unused_variables)]
        if let Some(config) = config {
            #[cfg(all(feature = "openssl", feature = "rustls"))]
            compile_error!("Feature openssl and Feature rustls can not be enabled together");

            #[cfg(any(feature = "openssl", feature = "rustls"))]
            {
                #[cfg(feature = "rustls")]
                {
                    let certificate = std::fs::read(config.certificate)?;
                    let private_key = std::fs::read(config.private_key)?;
                    axum_server::bind_rustls(
                        addr,
                        RustlsConfig::from_pem(certificate, private_key).await?,
                    )
                }
                #[cfg(feature = "openssl")]
                {
                    axum_server::bind_openssl(
                        addr,
                        OpenSSLConfig::from_pem_file(config.certificate, config.private_key)
                            .context("openssl load pem file error")?,
                    )
                }
            }
            .serve(
                self.router
                    .into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await?;
        } else {
            axum_server::bind(addr)
                .serve(
                    self.router
                        .into_make_service_with_connect_info::<SocketAddr>(),
                )
                .await
                .context("serve server error")?;
        }

        Ok(())
    }

    /// Setting up server router, see example for usage.
    ///
    pub fn route(mut self, path: impl AsRef<str>, router: Router) -> Self {
        self.router = self.router.nest(path.as_ref(), router);
        self
    }

    /// Server listening port, default is 8080
    ///
    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Setting up a runtime static file path.
    ///
    /// Unlike [spa_server_root], file in this path can be changed in runtime.
    pub fn static_path(mut self, path: impl Into<String>, dir: impl Into<PathBuf>) -> Self {
        self.static_path.push((path.into(), dir.into()));
        self
    }

    /// add host based router
    ///
    pub fn host_router(mut self, host: impl Into<String>, router: Router) -> Self {
        self.host_routers.insert(host.into(), router);
        self
    }

    fn add_cache_control() -> SetResponseHeaderLayer<HeaderValue> {
        SetResponseHeaderLayer::if_not_present(
            header::CACHE_CONTROL,
            HeaderValue::from_static("max-age=300"),
        )
    }
}

#[derive(Clone)]
struct MatchHostLayer {
    host_routers: Arc<HashMap<String, Router>>,
}

impl<S> Layer<S> for MatchHostLayer {
    type Service = MatchHost<S>;

    fn layer(&self, inner: S) -> Self::Service {
        MatchHost {
            inner,
            host_routers: self.host_routers.clone(),
        }
    }
}

impl MatchHostLayer {
    pub fn new(host_routers: Arc<HashMap<String, Router>>) -> Self {
        Self { host_routers }
    }
}

#[derive(Clone)]
struct MatchHost<S> {
    inner: S,
    host_routers: Arc<HashMap<String, Router>>,
}

impl<S> Service<Request> for MatchHost<S>
where
    S: Service<Request, Response = Response, Error = Infallible> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Into<Infallible>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<S::Response, S::Error>>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request) -> Self::Future {
        let host_routers = self.host_routers.clone();
        let mut srv = self.inner.clone();
        Box::pin(async move {
            let hostname = req
                .headers()
                .get(header::HOST)
                .and_then(|h| h.to_str().ok())
                .unwrap_or_default();

            if let Some((_, router)) = host_routers.iter().find(|(k, _v)| hostname.ends_with(*k)) {
                router.clone().oneshot(req).await
            } else {
                srv.call(req).await
            }
        })
    }
}

pub struct HttpsConfig {
    pub certificate: PathBuf,
    pub private_key: PathBuf,
}

/// setup https pems   
///
/// ## Example
/// ```
/// https_pems!("/some/folder/contains/two/pem/file");
/// ```
///
/// ## Caution
/// pem file name should be [`cert.pem`] and [`key.pem`]
///
#[macro_export]
macro_rules! embed_https_pems {
    ($path: literal) => {
        #[derive(spa_rs::rust_embed::RustEmbed)]
        #[crate_path = "spa_rs::rust_embed"]
        #[folder = $path]
        struct HttpsPems;
    };

    () => {{
        let https_config = || -> anyhow::Result<spa_rs::HttpsConfig> {
            let mut base_path = std::env::temp_dir().join(format!(
                "{}_{}",
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION")
            ));
            let _ = std::fs::create_dir_all(&base_path);
            let mut cert_path = None;
            let mut key_path = None;
            for file in HttpsPems::iter() {
                if let Some(f) = HttpsPems::get(&file) {
                    if file == "key.pem" {
                        key_path = Some(base_path.join("key.pem"));
                        std::fs::write(key_path.as_ref().unwrap(), &f.data)?;
                    }

                    if file == "cert.pem" {
                        cert_path = Some(base_path.join("cert.pem"));
                        std::fs::write(cert_path.as_ref().unwrap(), &f.data)?;
                    }
                }
            }

            if cert_path.is_none() || key_path.is_none() {
                anyhow::bail!("invalid ssl cert or key embed file");
            }

            Ok(spa_rs::HttpsConfig {
                certificate: cert_path.unwrap(),
                private_key: key_path.unwrap(),
            })
        };
        https_config()
    }};
}

/// Specific SPA dist file root path in compile time
///
#[macro_export]
macro_rules! spa_server_root {
    ($root: literal) => {
        #[derive(spa_rs::rust_embed::RustEmbed)]
        #[crate_path = "spa_rs::rust_embed"]
        #[folder = $root]
        struct StaticFiles;

        impl spa_rs::SpaStatic for StaticFiles {}
    };
    () => {
        StaticFiles
    };
}

/// Used to release static file into temp dir in runtime.
///
pub trait SpaStatic: rust_embed::RustEmbed {
    fn release(&self, release_path: PathBuf) -> Result<PathBuf> {
        let target_dir = release_path;
        if !target_dir.exists() {
            create_dir_all(&target_dir)?;
        }

        for file in Self::iter() {
            match Self::get(&file) {
                Some(f) => {
                    if let Some(p) = std::path::Path::new(file.as_ref()).parent() {
                        let parent_dir = target_dir.join(p);
                        create_dir_all(parent_dir)?;
                    }

                    let path = target_dir.join(file.as_ref());
                    debug!("release static file: {}", path.display());
                    if let Err(e) = fs::write(path, f.data) {
                        error!("static file {} write error: {:?}", file, e);
                    }
                }
                None => warn!("static file {} not found", file),
            }
        }

        Ok(target_dir)
    }
}

impl SpaStatic for ApiOnly {}
impl rust_embed::RustEmbed for ApiOnly {
    fn get(_file_path: &str) -> Option<rust_embed::EmbeddedFile> {
        unreachable!()
    }

    fn iter() -> rust_embed::Filenames {
        unreachable!()
    }
}

struct ApiOnly;
