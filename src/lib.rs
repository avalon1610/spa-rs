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
    body::Bytes,
    body::HttpBody,
    extract::{Host, Request},
    http::HeaderValue,
    response::Response,
    routing::{any, get_service, Route},
};
#[cfg(feature = "openssl")]
use axum_server::tls_openssl::OpenSSLConfig;
#[cfg(feature = "rustls")]
use axum_server::tls_rustls::RustlsConfig;
use http::{
    header::{self},
    StatusCode,
};
#[cfg(feature = "reverse-proxy")]
use http::{Method, Uri};
use log::{debug, error, warn};
use std::{
    collections::HashMap,
    convert::Infallible,
    env::current_exe,
    fs::{self, create_dir_all},
    net::SocketAddr,
    path::{Path, PathBuf},
};
use tower::{Layer, Service, ServiceExt as TowerServiceExt};
use tower_http::{
    services::{ServeDir, ServeFile},
    set_header::SetResponseHeaderLayer,
};

pub mod rust_embed {
    pub use rust_embed::*;
}

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
    main_router: Router,
    api_router: Router,
    data: Option<T>,
    forward: Option<String>,
    release_path: PathBuf,
    extra_layer: Vec<Box<dyn FnOnce(Router) -> Router>>,
    host_routers: HashMap<String, Router>,
}

#[cfg(feature = "reverse-proxy")]
async fn forwarded_to_dev(
    Extension(forward_addr): Extension<String>,
    uri: Uri,
    method: Method,
) -> HttpResult<Response> {
    compile_error!("Can not use now, wait for reqwest upgrade hyper to 1.0");
    // use http::uri::Scheme;

    // if method == Method::GET {
    //     let client = reqwest::Client::builder().no_proxy().build()?;
    //     let mut parts = uri.into_parts();
    //     parts.authority = Some(forward_addr.parse()?);
    //     if parts.scheme.is_none() {
    //         parts.scheme = Some(Scheme::HTTP);
    //     }
    //     let url = Uri::from_parts(parts)?.to_string();

    //     println!("forward url: {}", url);
    //     let response = client.get(url).send().await?;
    //     let status = response.status();
    //     let headers = response.headers().clone();
    //     let bytes = response.bytes().await?;

    //     let mut response = Response::builder().status(status);
    //     *(response.headers_mut().unwrap()) = headers;
    //     let response = response.body(bytes)?;
    //     return Ok(response);
    // }

    // Err(HttpError {
    //     message: "Method not allowed".to_string(),
    //     status_code: StatusCode::METHOD_NOT_ALLOWED,
    // })
    todo!()
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
            main_router: Router::new(),
            forward: None,
            release_path: current_exe()?
                .parent()
                .ok_or_else(|| anyhow!("no parent in current_exe"))?
                .join(format!(".{}_static_files", env!("CARGO_PKG_NAME"))),
            extra_layer: Vec::new(),
            host_routers: HashMap::new(),
            api_router: Router::new(),
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

            self.api_router = if let Some(addr) = self.forward {
                self.api_router
                    .fallback(forwarded_to_dev)
                    .layer(Extension(addr))
            } else {
                self.api_router.fallback_service(
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
            self.api_router = self.api_router.nest_service(
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

        let main_handler = |Host(hostname): Host, request: Request| async move {
            if let Some(router) = self.host_routers.remove(&hostname) {
                router.oneshot(request).await
            } else {
                self.api_router.oneshot(request).await
            }
        };
        self.main_router = Router::new()
            .route("/", any(main_handler.clone()))
            .route("/*path", any(main_handler));

        if let Some(data) = self.data {
            self.main_router = self.main_router.layer(Extension(data));
        }

        for layer in self.extra_layer {
            self.main_router = layer(self.main_router)
        }

        let addr = format!("0.0.0.0:{}", self.port).parse()?;
        if let Some(_config) = config {
            #[cfg(all(feature = "openssl", feature = "rustls"))]
            compile_error!("Feature openssl and Feature rustls can not be enabled together");

            #[cfg(any(feature = "openssl", feature = "rustls"))]
            {
                #[cfg(feature = "rustls")]
                {
                    axum_server::bind_rustls(
                        addr,
                        RustlsConfig::from_pem(_config.certificate, _config.private_key).await?,
                    )
                }
                #[cfg(feature = "openssl")]
                {
                    let temp_dir = std::env::temp_dir().join(env!("CARGO_PKG_NAME"));
                    std::fs::create_dir_all(&temp_dir)?;
                    let cert_file = temp_dir.join("cert.pem");
                    let key_file = temp_dir.join("key.pem");
                    std::fs::write(&cert_file, &_config.certificate)?;
                    std::fs::write(&key_file, &_config.private_key)?;
                    axum_server::bind_openssl(
                        addr,
                        OpenSSLConfig::from_pem_file(cert_file, key_file)
                            .context("openssl load pem file error")?,
                    )
                }
            }
            .serve(
                self.main_router
                    .into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await?;
        } else {
            axum_server::bind(addr)
                .serve(
                    self.main_router
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
        self.api_router = self.api_router.nest(path.as_ref(), router);
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

pub struct HttpsConfig {
    pub certificate: Vec<u8>,
    pub private_key: Vec<u8>,
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
macro_rules! https_pems {
    ($path: literal) => {
        #[derive(spa_rs::rust_embed::RustEmbed)]
        #[folder = $path]
        struct HttpsPems;
    };

    () => {{
        let https_config = || -> anyhow::Result<spa_rs::HttpsConfig> {
            let mut cert = Vec::new();
            let mut key = Vec::new();
            for file in HttpsPems::iter() {
                if let Some(f) = HttpsPems::get(&file) {
                    macro_rules! setup {
                        ($t: expr) => {
                            if file == format!("{}.pem", stringify!($t)) {
                                $t = f.data.to_vec();
                            }
                        };
                    }
                    setup!(cert);
                    setup!(key);
                }
            }

            if cert.is_empty() || key.is_empty() {
                anyhow::bail!("invalid ssl cert or key embed file");
            }

            Ok(spa_rs::HttpsConfig {
                certificate: cert,
                private_key: key,
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
        use spa_rs::rust_embed;

        #[derive(rust_embed::RustEmbed)]
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
impl rust_embed::RustEmbed for ApiOnly {
    fn get(_file_path: &str) -> Option<rust_embed::EmbeddedFile> {
        unreachable!()
    }

    fn iter() -> rust_embed::Filenames {
        unreachable!()
    }
}

struct ApiOnly;
