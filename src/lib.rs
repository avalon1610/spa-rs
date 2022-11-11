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
use anyhow::{anyhow, Result};
#[cfg(feature = "reverse-proxy")]
use axum::response::IntoResponse;
use axum::{
    body::Bytes,
    body::{Body, HttpBody},
    handler::Handler,
    http::HeaderValue,
    response::Response,
    routing::{get_service, Route, Router},
};
use axum_server::tls_rustls::RustlsConfig;
#[cfg(feature = "reverse-proxy")]
use http::Method;
use http::{header, Request, StatusCode};
use log::{debug, error, warn};
use std::{
    convert::Infallible,
    env::current_exe,
    fs::{self, create_dir_all},
    path::{Path, PathBuf}, net::SocketAddr,
};
use tower::{Layer, Service};
use tower_http::{
    services::{ServeDir, ServeFile},
    set_header::SetResponseHeaderLayer,
};

pub use axum::*;
pub use http_body;
pub use rust_embed::RustEmbed;

pub mod auth;
pub mod session;
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
pub struct SpaServer {
    static_path: Option<(String, PathBuf)>,
    port: u16,
    app: Router,
    forward: Option<String>,
    release_path: PathBuf,
    extra_layer: Vec<Box<dyn FnOnce(Router) -> Router>>,
}

#[cfg(feature = "reverse-proxy")]
async fn forwarded_to_dev(
    Extension(forward_addr): Extension<String>,
    uri: Uri,
    method: Method,
) -> HttpResult<impl IntoResponse> {
    use axum::body::Full;
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
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.bytes().await?;

        let mut response = Response::builder().status(status);
        *(response.headers_mut().unwrap()) = headers;
        let response = response.body(Full::from(bytes))?;
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

impl SpaServer {
    /// Just new(), nothing special
    pub fn new() -> Result<Self> {
        Ok(Self {
            static_path: None,
            port: 8080,
            app: Router::new(),
            forward: None,
            release_path: current_exe()?
                .parent()
                .ok_or_else(|| anyhow!("no parent in current_exe"))?
                .join(format!(".{}_static_files", env!("CARGO_PKG_NAME"))),
            extra_layer: Vec::new(),
        })
    }

    /// Specific server context data
    ///
    /// This is similar to [axum middleware](https://docs.rs/axum/latest/axum/#middleware)
    pub fn data<T>(mut self, data: T) -> Self
    where
        T: Clone + Send + Sync + 'static,
    {
        self.app = self.app.layer(Extension(data));
        self
    }

    /// Specific an axum layer to server
    ///
    /// This is similar to [axum middleware](https://docs.rs/axum/latest/axum/#middleware)
    pub fn layer<L, NewResBody>(mut self, layer: L) -> Self
    where
        L: Layer<Route> + 'static,
        L::Service: Service<Request<Body>, Response = Response<NewResBody>, Error = Infallible>
            + Clone
            + Send
            + 'static,
        <L::Service as Service<Request<Body>>>::Future: Send + 'static,
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
    pub async fn run_tls<Root>(self, root: Root, config: HttpsConfig) -> Result<()>
    where
        Root: SpaStatic,
    {
        self.run_raw(Some(root), Some(config)).await
    }

    /// Run the spa server without spa root
    pub async fn run_api<Root>(self) -> Result<()>
    where
        Root: SpaStatic,
    {
        self.run_raw::<ApiOnly>(None, None).await
    }

    /// Run the spa server with tls and without spa root
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

            self.app = if let Some(addr) = self.forward {
                self.app
                    .fallback(forwarded_to_dev.into_service())
                    .layer(Extension(addr))
            } else {
                self.app.fallback(
                    get_service(ServeDir::new(&embeded_dir).fallback(ServeFile::new(&index_file)))
                        .layer(Self::add_cache_control())
                        .handle_error(|e| async move {
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

        if let Some(sf) = self.static_path {
            self.app = self.app.nest(
                &sf.0,
                get_service(ServeDir::new(&sf.1))
                    .layer(Self::add_cache_control())
                    .handle_error(|e| async move {
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

        for layer in self.extra_layer {
            self.app = layer(self.app)
        }

        let addr = format!("0.0.0.0:{}", self.port).parse()?;
        if let Some(config) = config {
            axum_server::bind_rustls(
                addr,
                RustlsConfig::from_pem(config.certificate, config.private_key).await?,
            )
            .serve(self.app.into_make_service_with_connect_info::<SocketAddr>())
            .await?;
        } else {
            axum_server::bind(addr)
                .serve(self.app.into_make_service_with_connect_info::<SocketAddr>())
                .await?;
        }

        Ok(())
    }

    /// Setting up server router, see example for usage.
    ///
    pub fn route(mut self, path: impl AsRef<str>, router: Router) -> Self {
        self.app = self.app.nest(path.as_ref(), router);
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
        self.static_path = Some((path.into(), dir.into()));
        self
    }

    fn add_cache_control() -> SetResponseHeaderLayer<HeaderValue> {
        SetResponseHeaderLayer::if_not_present(
            header::CACHE_CONTROL,
            HeaderValue::from_static("max-age=300"),
        )
    }
}

pub struct HttpsConfig {
    pub certificate: Vec<u8>,
    pub private_key: Vec<u8>,
}

/// Specific SPA dist file root path in compile time
///
#[macro_export]
macro_rules! spa_server_root {
    ($root: literal) => {
        #[derive(spa_rs::RustEmbed)]
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
pub trait SpaStatic: RustEmbed {
    fn release(&self, release_path: PathBuf) -> Result<PathBuf> {
        let target_dir = release_path;
        if !target_dir.exists() {
            create_dir_all(&target_dir)?;
        }

        for file in Self::iter() {
            match Self::get(&file) {
                Some(f) => {
                    if let Some(p) = Path::new(file.as_ref()).parent() {
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
impl RustEmbed for ApiOnly {
    fn get(_file_path: &str) -> Option<rust_embed::EmbeddedFile> {
        unreachable!()
    }

    fn iter() -> rust_embed::Filenames {
        unreachable!()
    }
}

struct ApiOnly;
