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
//!     let mut srv = SpaServer::new();
//!     srv.port(3000)
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
use anyhow::Result;
#[cfg(feature = "reverse-proxy")]
use axum::response::IntoResponse;
use axum::{
    handler::Handler,
    http::HeaderValue,
    routing::{get_service, Router},
};
#[cfg(feature = "reverse-proxy")]
use http::Method;
use http::{header, StatusCode, Uri};
use log::{debug, error, warn};
use std::{
    env::temp_dir,
    fs::{self, create_dir_all},
    net::SocketAddr,
    path::{Path, PathBuf},
};
use tower_http::{
    services::{ServeDir, ServeFile},
    set_header::SetResponseHeaderLayer,
};

pub use axum::*;
pub use rust_embed::RustEmbed;

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
pub struct SpaServer<T> {
    static_path: Option<(String, PathBuf)>,
    data: Option<T>,
    port: u16,
    routes: Vec<(String, Router)>,
    forward: Option<Uri>,
    release_path: PathBuf,
}

#[cfg(feature = "reverse-proxy")]
async fn forwarded_to_dev(
    Extension(proxy_uri): Extension<Uri>,
    uri: Uri,
    method: Method,
) -> HttpResult<impl IntoResponse> {
    use axum::{body::Full, response::Response};

    if method == Method::GET {
        let client = reqwest::Client::builder().no_proxy().build()?;
        let url = format!(
            "{}{}",
            proxy_uri.to_string().trim_end_matches('/'),
            uri.to_string()
        );
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

impl<T> SpaServer<T>
where
    T: Clone + Sync + Send + 'static,
{
    /// Just new(), nothing special
    pub fn new() -> Self {
        Self {
            static_path: None,
            data: None,
            port: 8080,
            routes: Vec::new(),
            forward: None,
            release_path: temp_dir().join(format!("{}_static_files", env!("CARGO_PKG_NAME"))),
        }
    }

    /// Specific server context data
    ///
    /// This is similar to [axum middleware](https://docs.rs/axum/latest/axum/#middleware)
    pub fn data(&mut self, data: T) -> &mut Self {
        self.data = Some(data);
        self
    }

    /// make a reverse proxy which redirect all SPA requests to dev server, such as `ng serve`, `vite`.  
    ///
    /// it's useful when debugging UI
    #[cfg(feature = "reverse-proxy")]
    #[cfg_attr(docsrs, doc(cfg(feature = "reverse-proxy")))]
    pub fn reverse_proxy(&mut self, uri: Uri) -> &mut Self {
        self.forward = Some(uri);
        self
    }

    /// static file release path in runtime
    ///
    /// Default path is /tmp/[env!(CARGO_PKG_NAME)]_static_files
    pub fn release_path(&mut self, rp: impl Into<PathBuf>) -> &mut Self {
        self.release_path = rp.into();
        self
    }

    /// Run the spa server forever
    pub async fn run<Root>(self, root: Root) -> Result<()>
    where
        Root: SpaStatic,
    {
        let embeded_dir = root.release(self.release_path)?;
        let index_file = embeded_dir.clone().join("index.html");

        let mut app = Router::new();
        app = if let Some(uri) = self.forward {
            app.fallback(forwarded_to_dev.into_service())
                .layer(Extension(uri))
        } else {
            app.fallback(
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

        if let Some(sf) = self.static_path {
            app = app.nest(
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

        for route in self.routes {
            app = app.nest(&route.0, route.1);
        }

        if let Some(data) = self.data {
            app = app.layer(Extension(data));
        }

        Server::bind(&format!("0.0.0.0:{}", self.port).parse()?)
            .serve(app.into_make_service_with_connect_info::<SocketAddr>())
            .await?;

        Ok(())
    }

    /// Setting up server router, see example for usage.
    ///
    pub fn route(&mut self, path: impl Into<String>, router: Router) -> &mut Self {
        self.routes.push((path.into(), router));
        self
    }

    /// Server listening port, default is 8080
    ///
    pub fn port(&mut self, port: u16) -> &mut Self {
        self.port = port;
        self
    }

    /// Setting up a runtime static file path.
    ///
    /// Unlike [spa_server_root], file in this path can be changed in runtime.
    pub fn static_path(&mut self, path: impl Into<String>, dir: impl Into<PathBuf>) -> &mut Self {
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
