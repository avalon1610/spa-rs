//! spa-rs is a library who can embed all SPA web application files (dist static file),
//! and release as a single binary executable.
//!
//! It based-on [`axum`] and [`rust_embed`]
//!
//! It reexported all axum module for convenient use.
//! # Example
//! ```no_run
//! use spa_rs::spa_server_root;
//! use spa_rs::SpaServer;
//! use spa_rs::axum::routing::{get, Router};
//! use anyhow::Result;
//!
//! spa_server_root!("web/dist");           // specific your SPA dist file location
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     let srv = SpaServer::new()
//!         .port(3000)
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
use anyhow::{Context, Result, anyhow};
#[cfg(feature = "reverse-proxy")]
use axum::http::{Method, Uri};
use axum::{
    Extension,
    body::{Bytes, HttpBody},
    extract::Request,
    http::HeaderValue,
    response::Response,
    routing::{MethodRouter, Route, get_service},
};
use axum::{
    Router,
    http::{
        StatusCode,
        header::{self},
    },
};
#[cfg(feature = "openssl")]
use axum_server::tls_openssl::OpenSSLConfig;
#[cfg(feature = "rustls")]
use axum_server::tls_rustls::RustlsConfig;
use futures_util::future::BoxFuture;
use log::{debug, error, warn};
use rand::{Rng, distributions::Alphanumeric, thread_rng};
pub use rust_embed;
use std::{
    any::type_name,
    borrow::Cow,
    collections::HashMap,
    convert::Infallible,
    env::{current_exe, home_dir, temp_dir},
    fs::{self, create_dir_all},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tower::{Layer, Service, util::ServiceExt as TowerServiceExt};
use tower_http::{
    services::{ServeDir, ServeFile},
    set_header::SetResponseHeaderLayer,
};

pub use axum;
pub mod auth;
pub mod session;
pub use axum::debug_handler;
pub use axum_extra as extra;
pub use axum_help as help;

/// A server wrapped axum server.
///
/// It can:
/// - serve static files in SPA root path
/// - serve API requests in router
/// - fallback to SPA static file when route matching failed
///     - if still get 404, it will redirect to SPA index.html
///
#[derive(Default, Clone)]
pub struct SpaServer<T = ()>
where
    T: Clone + Send + Sync + 'static,
{
    static_path: Vec<(String, PathBuf)>,
    port: u16,
    router: Router<T>,
    data: Option<T>,
    state: Option<T>,
    forward: Option<String>,
    release_path: Option<PathBuf>,
    extra_layer: Vec<ExtraLayerFn>,
    host_routers: HashMap<String, Router<T>>,
}

type ExtraLayerFn = Arc<dyn Fn(Router<()>) -> Router<()> + Send + Sync>;

#[axum::debug_handler]
#[cfg(feature = "reverse-proxy")]
async fn forwarded_to_dev(
    axum::Extension(forward_addr): axum::Extension<String>,
    uri: axum::http::uri::Uri,
    method: Method,
) -> help::HttpResult<axum::response::Response> {
    if method == Method::GET {
        let client = reqwest::Client::builder().no_proxy().build()?;
        let mut parts = uri.into_parts();
        parts.authority = Some(forward_addr.parse()?);
        if parts.scheme.is_none() {
            parts.scheme = Some(axum::http::uri::Scheme::HTTP);
        }
        let uri = Uri::from_parts(parts)?.to_string();

        println!("forward url: {uri}");
        let response = client.get(uri).send().await?;
        let response: axum::http::Response<_> = response.into();
        let (parts, body) = response.into_parts();
        let body = body.as_bytes().map(<[u8]>::to_vec).unwrap_or_default();

        let response = Response::from_parts(parts, body.into());
        return Ok(response);
    }

    Err(axum_help::HttpError {
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
    /// Just `new()`, nothing special
    #[must_use]
    pub fn new() -> Self {
        Self {
            static_path: Vec::new(),
            port: 8080,
            forward: None,
            release_path: None,
            extra_layer: Vec::new(),
            host_routers: HashMap::new(),
            router: Router::new(),
            data: None,
            state: None,
        }
    }

    /// Specific server context data
    ///
    /// SOFT Deprecated, use `.state()` instead.
    #[must_use]
    pub fn data(mut self, data: T) -> Self
    where
        T: Clone + Send + Sync + 'static,
    {
        self.data = Some(data);
        self
    }

    /// Sepcific server state, which can be modified in runtime.
    ///
    #[must_use]
    pub fn state(mut self, st: T) -> Self
    where
        T: Clone + Send + Sync + 'static,
    {
        self.state = Some(st);
        self
    }

    /// Specific an axum layer to server
    ///
    /// This is similar to [axum middleware](https://docs.rs/axum/latest/axum/#middleware)
    #[must_use]
    pub fn layer<L, NewResBody>(mut self, layer: L) -> Self
    where
        L: Layer<Route> + Clone + Send + Sync + 'static,
        L::Service: Service<Request, Response = Response<NewResBody>, Error = Infallible>
            + Clone
            + Send
            + Sync
            + 'static,
        <L::Service as Service<Request>>::Future: Send + 'static,
        NewResBody: HttpBody<Data = Bytes> + Send + 'static,
        NewResBody::Error: Into<axum::BoxError>,
    {
        self.extra_layer
            .push(Arc::new(move |app| app.layer(layer.clone())));
        self
    }

    /// make a reverse proxy which redirect all SPA requests to dev server, such as `ng serve`, `vite`.  
    ///
    /// it's useful when debugging UI
    #[cfg(feature = "reverse-proxy")]
    #[cfg_attr(docsrs, doc(cfg(feature = "reverse-proxy")))]
    #[must_use]
    pub fn reverse_proxy(mut self, addr: impl Into<String>) -> Self {
        self.forward = Some(addr.into());
        self
    }

    /// static file release path in runtime
    ///
    /// Default path is `$HOME/.[executable name]_static_files_[hash]`, where the hash
    /// is derived from the names and contents of the embedded files. Without a home
    /// directory the directory goes to the system temp dir with a random suffix
    /// instead, since a predictable name there could be pre-created by another local
    /// user. The same binary therefore always releases to the same directory across
    /// restarts, different applications never share one, and a rebuild with changed
    /// assets gets a fresh directory.
    ///
    /// Setting this to a fixed path disables that content addressing: files from
    /// removed or renamed assets stay in the directory, and no cleanup happens.
    /// The library never deletes release directories in either case.
    #[must_use]
    pub fn release_path(mut self, rp: impl Into<PathBuf>) -> Self {
        self.release_path = Some(rp.into());
        self
    }

    /// Run the spa server forever with a custom state.
    ///
    /// `.state(t)` must have been called before `run_with_state()`.
    /// If you don't need custom state, use `SpaServer::new()` (which is `SpaServer<()>`)
    /// and call `.run()` instead.
    pub async fn run_with_state<Root>(self, root: Root) -> Result<()>
    where
        Root: SpaStatic,
    {
        self.run_raw(Some(root), None).await
    }

    /// Run the spa server with tls and a custom state
    #[cfg(any(feature = "openssl", feature = "rustls"))]
    pub async fn run_tls_with_state<Root>(self, root: Root, config: HttpsConfig) -> Result<()>
    where
        Root: SpaStatic,
    {
        self.run_raw(Some(root), Some(config)).await
    }

    /// Run the spa server without spa root, with a custom state
    pub async fn run_api_with_state(self) -> Result<()> {
        self.run_raw::<ApiOnly>(None, None).await
    }

    /// Run the spa server with tls and without spa root, with a custom state
    #[cfg(any(feature = "openssl", feature = "rustls"))]
    pub async fn run_api_tls_with_state(self, config: HttpsConfig) -> Result<()> {
        self.run_raw::<ApiOnly>(None, Some(config)).await
    }

    /// Build up the router with fallbacks, static paths, and data extension.
    /// Returns the configured `SpaServer` ready for state conversion and serving.
    fn build_router<Root>(mut self, root: Option<Root>) -> Result<Self>
    where
        Root: SpaStatic,
    {
        if let Some(root) = root {
            let release_path = match self.release_path.take() {
                Some(path) => path,
                None => default_release_path::<Root>(),
            };
            let embeded_dir = root.release(release_path)?;
            let index_file = embeded_dir.clone().join("index.html");

            self.router = if let Some(ref addr) = self.forward {
                self.router
                    .fallback(forwarded_to_dev)
                    .layer(Extension(addr.clone()))
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

        for sf in self.static_path.drain(..) {
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
            );
        }

        if let Some(ref data) = self.data {
            self.router = self.router.layer(Extension(data.clone()));
        }

        Ok(self)
    }

    /// Serve a Router<()> with optional TLS config
    async fn serve_router(
        router: Router<()>,
        port: u16,
        config: Option<HttpsConfig>,
    ) -> Result<()> {
        let addr = format!("0.0.0.0:{port}").parse()?;
        #[allow(unused_variables)]
        if let Some(config) = config {
            #[cfg(all(feature = "openssl", feature = "rustls"))]
            compile_error!("Feature openssl and Feature rustls can not be enabled together");

            #[cfg(any(feature = "openssl", feature = "rustls"))]
            {
                #[cfg(feature = "rustls")]
                {
                    rustls::crypto::aws_lc_rs::default_provider()
                        .install_default()
                        .map_err(|e| {
                            std::io::Error::other(format!(
                                "install default crypto provider error: {e:?}"
                            ))
                        })?;
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
            .serve(router.into_make_service_with_connect_info::<SocketAddr>())
            .await?;
        } else {
            axum_server::bind(addr)
                .serve(router.into_make_service_with_connect_info::<SocketAddr>())
                .await
                .context("serve server error")?;
        }

        Ok(())
    }

    /// Run the spa server with or without spa root, and with or without tls.
    /// Requires state to have been provided via `.state()`.
    async fn run_raw<Root>(self, root: Option<Root>, config: Option<HttpsConfig>) -> Result<()>
    where
        Root: SpaStatic,
    {
        let this = self.build_router(root)?;

        let state = this.state.ok_or_else(|| {
            anyhow!(
                "SpaServer<{}> requires .state() to be called before run_with_state(). \
                 If you don't need custom state, use SpaServer::new() (which is SpaServer<()>).",
                type_name::<T>()
            )
        })?;

        let host_routers_unit: HashMap<String, Router<()>> = this
            .host_routers
            .into_iter()
            .map(|(k, v)| (k, v.with_state(state.clone())))
            .collect();

        let mut router: Router<()> = this.router.with_state(state);
        router = router.layer(MatchHostLayer::new(Arc::new(host_routers_unit)));

        for layer in this.extra_layer {
            router = layer(router);
        }

        Self::serve_router(router, this.port, config).await
    }

    /// Setting up server router, see example for usage.
    ///
    #[must_use]
    pub fn route(mut self, path: impl AsRef<str>, router: Router<T>) -> Self {
        self.router = self.router.nest(path.as_ref(), router);
        self
    }

    /// return the server router for modifying
    ///
    #[must_use]
    pub fn route_method(mut self, path: impl AsRef<str>, method: MethodRouter<T>) -> Self {
        let path = path.as_ref();
        self.router = self.router.route(path, method);
        self
    }

    /// Server listening port, default is 8080
    ///
    #[must_use]
    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Setting up a runtime static file path.
    ///
    /// Unlike [`spa_server_root`], file in this path can be changed in runtime.
    #[must_use]
    pub fn static_path(mut self, path: impl Into<String>, dir: impl Into<PathBuf>) -> Self {
        self.static_path.push((path.into(), dir.into()));
        self
    }

    /// add host based router
    ///
    #[must_use]
    pub fn host_router(mut self, host: impl Into<String>, router: Router<T>) -> Self {
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

impl SpaServer<()> {
    /// Run the spa server forever.
    ///
    /// No custom state is needed — `Router<()>` is served directly.
    /// For custom state, use `SpaServer::<YourState>::new()` and `.run_with_state()`.
    pub async fn run<Root>(self, root: Root) -> Result<()>
    where
        Root: SpaStatic,
    {
        self.run_raw_stateless(Some(root), None).await
    }

    /// Run the spa server with tls
    #[cfg(any(feature = "openssl", feature = "rustls"))]
    pub async fn run_tls<Root>(self, root: Root, config: HttpsConfig) -> Result<()>
    where
        Root: SpaStatic,
    {
        self.run_raw_stateless(Some(root), Some(config)).await
    }

    /// Run the spa server without spa root
    pub async fn run_api(self) -> Result<()> {
        self.run_raw_stateless::<ApiOnly>(None, None).await
    }

    /// Run the spa server with tls and without spa root
    #[cfg(any(feature = "openssl", feature = "rustls"))]
    pub async fn run_api_tls(self, config: HttpsConfig) -> Result<()> {
        self.run_raw_stateless::<ApiOnly>(None, Some(config)).await
    }

    /// Stateless run: Router<()> can be served directly without `.with_state()`.
    async fn run_raw_stateless<Root>(
        self,
        root: Option<Root>,
        config: Option<HttpsConfig>,
    ) -> Result<()>
    where
        Root: SpaStatic,
    {
        let this = self.build_router(root)?;

        // T = (), so host_routers are HashMap<String, Router<()>>
        let host_routers_unit: HashMap<String, Router<()>> = this
            .host_routers
            .into_iter()
            .map(|(k, v)| (k, v.with_state(())))
            .collect();

        // Router<()> .with_state(()) eagerly resolves handler closures
        let mut router: Router<()> = this.router.with_state(());
        router = router.layer(MatchHostLayer::new(Arc::new(host_routers_unit)));

        for layer in this.extra_layer {
            router = layer(router);
        }

        Self::serve_router(router, this.port, config).await
    }
}

#[derive(Clone)]
struct MatchHostLayer {
    host_routers: Arc<HashMap<String, Router<()>>>,
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
    pub fn new(host_routers: Arc<HashMap<String, Router<()>>>) -> Self {
        Self { host_routers }
    }
}

#[derive(Clone)]
struct MatchHost<S> {
    inner: S,
    host_routers: Arc<HashMap<String, Router<()>>>,
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
/// ```rust,ignore
/// # use spa_rs::embed_https_pems;
///
/// embed_https_pems!("/some/folder/contains/two/pem/file");
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
/// Extraction is atomic per file (write to a temporary file, then rename), so a
/// process serving the directory never observes a partially written file. The
/// directory is claimed with a marker file holding the identity of the embedded
/// file set; when the marker matches and every file is present, a restart skips
/// extraction entirely.
pub trait SpaStatic: rust_embed::RustEmbed {
    fn release(&self, release_path: PathBuf) -> Result<PathBuf>
    where
        Self: Sized,
    {
        let target_dir = release_path;
        create_dir_all(&target_dir)?;

        let id = release_id::<Self>();
        let marker = target_dir.join(RELEASE_MARKER);
        if fs::read_to_string(&marker).is_ok_and(|existing| existing.trim() == id)
            && Self::iter().all(|file| target_dir.join(file.as_ref()).is_file())
        {
            debug!("static files already released in {}", target_dir.display());
            return Ok(target_dir);
        }

        remove_stale_temp_files(&target_dir);

        let mut failed = false;
        for file in Self::iter() {
            let Some(f) = Self::get(&file) else {
                warn!("static file {file} not found");
                continue;
            };

            let path = target_dir.join(file.as_ref());
            debug!("release static file: {}", path.display());
            if let Err(e) = write_atomic(&path, &target_dir, &f.data) {
                error!("static file {file} write error: {e:?}");
                failed = true;
            }
        }

        // Only claim the directory once every file landed; a marker written after a
        // partial extraction would make the next start skip the missing files.
        if failed {
            let _ = fs::remove_file(&marker);
        } else if let Err(e) = write_atomic(&marker, &target_dir, id.as_bytes()) {
            error!("write release marker {} error: {e:?}", marker.display());
        }

        Ok(target_dir)
    }
}

/// Marker recording which set of embedded files a release directory holds.
const RELEASE_MARKER: &str = ".spa-rs-release";

/// Identity of the embedded file set: stable across restarts of the same build,
/// different whenever any file name or content changes.
fn release_id<Root: rust_embed::RustEmbed>() -> String {
    release_id_of(Root::iter().map(|file| {
        let hash = Root::get(&file).map_or([0; 32], |f| f.metadata.sha256_hash());
        (file, hash)
    }))
}

fn release_id_of(files: impl Iterator<Item = (Cow<'static, str>, [u8; 32])>) -> String {
    let mut files: Vec<_> = files.collect();
    files.sort_unstable_by(|a, b| a.0.cmp(&b.0));

    let mut ctx = md5::Context::new();
    for (name, hash) in files {
        ctx.consume(name.len().to_le_bytes());
        ctx.consume(name.as_bytes());
        ctx.consume(hash);
    }

    let hex = format!("{:x}", ctx.compute());
    hex[..12].to_owned()
}

/// Default release directory: one stable directory per application *and* per
/// embedded file set, so restarts reuse it while concurrent processes and
/// unrelated applications never share one.
///
/// Without a home directory the directory is placed in the system temp dir and gets
/// a random suffix instead: `/tmp` is shared between users, so a predictable name
/// there could be pre-created by another local user (symlink or permission denial)
/// before this process ever starts.
fn default_release_path<Root: rust_embed::RustEmbed>() -> PathBuf {
    let name = current_exe()
        .ok()
        .and_then(|exe| exe.file_stem().and_then(|s| s.to_str()).map(str::to_owned))
        .unwrap_or_else(|| "spa".to_owned());
    let id = release_id::<Root>();

    match home_dir() {
        Some(home) => home.join(release_dir_name(&name, &id)),
        None => temp_dir().join(format!(
            "{}-{}",
            release_dir_name(&name, &id),
            rand_string(8)
        )),
    }
}

/// Directory name for an application called `app` holding file set `id`.
fn release_dir_name(app: &str, id: &str) -> String {
    format!(".{app}_static_files_{id}")
}

/// Write `data` to `target` via a temporary file in `tmp_dir`, so a reader never
/// observes a half-written file.
fn write_atomic(target: &Path, tmp_dir: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = target.parent() {
        create_dir_all(parent)?;
    }

    let tmp = tmp_dir.join(format!(".spa-rs-{}.tmp", rand_string(12)));
    fs::write(&tmp, data)?;

    // Windows cannot rename over an existing file; Unix rename replaces atomically.
    if cfg!(windows) && target.exists() {
        let _ = fs::remove_file(target);
    }

    if let Err(e) = fs::rename(&tmp, target) {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }

    Ok(())
}

/// Drop temporary files left behind by an interrupted release.
///
/// Only files older than [`STALE_TEMP_AGE`] are touched: a younger one may belong to
/// another process that is extracting right now.
fn remove_stale_temp_files(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let age = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok());
        if is_abandoned_temp_file(&entry.file_name(), age) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Temp files this crate writes are abandoned once they are older than this; a
/// younger one may still be in flight in another process.
const STALE_TEMP_AGE: Duration = Duration::from_secs(3600);

/// Whether `name` is one of our temporary files and old enough to be reaped.
///
/// Unknown age (clock skew, unsupported metadata) is treated as fresh, so a live
/// extraction is never deleted on a guess.
fn is_abandoned_temp_file(name: &std::ffi::OsStr, age: Option<Duration>) -> bool {
    let path = Path::new(name);
    let is_ours = path.extension().is_some_and(|ext| ext == "tmp")
        && path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(|stem| stem.starts_with(".spa-rs-"));
    is_ours && age.is_some_and(|age| age > STALE_TEMP_AGE)
}

impl SpaStatic for ApiOnly {}
#[allow(refining_impl_trait)]
impl rust_embed::RustEmbed for ApiOnly {
    fn get(_file_path: &str) -> Option<rust_embed::EmbeddedFile> {
        unreachable!()
    }

    fn iter() -> rust_embed::Filenames {
        unreachable!()
    }
}

struct ApiOnly;

fn rand_string(count: usize) -> String {
    thread_rng()
        .sample_iter(Alphanumeric)
        .take(count)
        .map(char::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(files: &[(&'static str, [u8; 32])]) -> String {
        release_id_of(
            files
                .iter()
                .map(|(name, hash)| (Cow::Borrowed(*name), *hash)),
        )
    }

    #[test]
    fn release_id_is_stable_and_order_independent() {
        let a = ("a.js", [1; 32]);
        let b = ("index.html", [2; 32]);

        assert_eq!(id(&[a, b]), id(&[a, b]));
        assert_eq!(id(&[a, b]), id(&[b, a]));
        assert_eq!(id(&[a, b]).len(), 12);
    }

    #[test]
    fn release_id_changes_with_name_or_content() {
        let base = id(&[("a.js", [1; 32])]);

        assert_ne!(base, id(&[("a.js", [2; 32])]), "content change");
        assert_ne!(base, id(&[("b.js", [1; 32])]), "name change");
        assert_ne!(
            base,
            id(&[("a.js", [1; 32]), ("b.js", [3; 32])]),
            "extra file"
        );
    }

    #[test]
    fn release_id_does_not_collide_on_shifted_boundaries() {
        // Without length prefixes `ab` + `c` hashes the same bytes as `a` + `bc`.
        assert_ne!(
            id(&[("ab", [0; 32]), ("c", [1; 32])]),
            id(&[("a", [0; 32]), ("bc", [1; 32])])
        );
    }

    #[test]
    fn default_release_path_is_stable_and_app_specific() {
        let first = default_release_path::<TestFiles>();
        let second = default_release_path::<TestFiles>();
        assert_eq!(first, second, "restarting the same app must reuse its dir");

        let release = release_id::<TestFiles>();
        let name = release_dir_name("myapp", &release);
        assert_eq!(name, format!(".myapp_static_files_{release}"));
        assert_ne!(
            name,
            release_dir_name("otherapp", &release),
            "apps must not share"
        );
        assert_ne!(
            name,
            release_dir_name("myapp", &id(&[("index.html", [9; 32])])),
            "new assets must not reuse a stale dir"
        );
    }

    #[test]
    fn release_reuses_directory_across_calls() {
        let dir = temp_dir().join(format!("spa-rs-test-{}", rand_string(12)));
        let _cleanup = Cleanup(dir.clone());

        TestFiles.release(dir.clone()).unwrap();
        let marker = fs::read_to_string(dir.join(RELEASE_MARKER)).unwrap();
        let first_mtime = fs::metadata(dir.join("index.html"))
            .unwrap()
            .modified()
            .unwrap();

        TestFiles.release(dir.clone()).unwrap();
        assert_eq!(
            fs::read_to_string(dir.join(RELEASE_MARKER)).unwrap(),
            marker
        );
        assert_eq!(
            fs::metadata(dir.join("index.html"))
                .unwrap()
                .modified()
                .unwrap(),
            first_mtime,
            "second release must not rewrite already-released files"
        );
        assert_eq!(
            fs::read_to_string(dir.join("index.html")).unwrap(),
            "test index"
        );
    }

    #[test]
    fn release_repairs_directory_with_wrong_marker() {
        let dir = temp_dir().join(format!("spa-rs-test-{}", rand_string(12)));
        let _cleanup = Cleanup(dir.clone());

        create_dir_all(&dir).unwrap();
        fs::write(dir.join(RELEASE_MARKER), "stale-id").unwrap();
        TestFiles.release(dir.clone()).unwrap();

        assert_eq!(
            fs::read_to_string(dir.join(RELEASE_MARKER)).unwrap(),
            release_id::<TestFiles>()
        );
        assert_eq!(
            fs::read_to_string(dir.join("index.html")).unwrap(),
            "test index"
        );
    }

    #[test]
    fn release_repairs_directory_with_missing_file() {
        let dir = temp_dir().join(format!("spa-rs-test-{}", rand_string(12)));
        let _cleanup = Cleanup(dir.clone());

        TestFiles.release(dir.clone()).unwrap();
        fs::remove_file(dir.join("index.html")).unwrap();
        TestFiles.release(dir.clone()).unwrap();

        assert_eq!(
            fs::read_to_string(dir.join("index.html")).unwrap(),
            "test index"
        );
    }

    #[test]
    fn temp_files_are_only_reaped_once_abandoned() {
        let name = std::ffi::OsStr::new(".spa-rs-abc123.tmp");

        assert!(!is_abandoned_temp_file(name, Some(Duration::from_secs(60))));
        assert!(!is_abandoned_temp_file(name, Some(STALE_TEMP_AGE)));
        assert!(
            !is_abandoned_temp_file(name, None),
            "unknown age must not be guessed at"
        );
        assert!(is_abandoned_temp_file(name, Some(STALE_TEMP_AGE * 2)));
    }

    #[test]
    fn temp_file_match_ignores_foreign_files() {
        let old = Some(STALE_TEMP_AGE * 2);

        assert!(!is_abandoned_temp_file(
            std::ffi::OsStr::new("index.html"),
            old
        ));
        assert!(!is_abandoned_temp_file(
            std::ffi::OsStr::new("app.tmp"),
            old
        ));
        assert!(!is_abandoned_temp_file(
            std::ffi::OsStr::new(".spa-rs-release"),
            old
        ));
    }

    #[test]
    fn release_keeps_in_flight_temp_file() {
        let dir = temp_dir().join(format!("spa-rs-test-{}", rand_string(12)));
        let _cleanup = Cleanup(dir.clone());

        create_dir_all(&dir).unwrap();
        let in_flight = dir.join(".spa-rs-active.tmp");
        fs::write(&in_flight, "partial").unwrap();

        TestFiles.release(dir.clone()).unwrap();

        assert!(
            in_flight.exists(),
            "a fresh temp file may belong to a concurrent extraction"
        );
    }

    #[test]
    fn release_creates_nested_directories() {
        let dir = temp_dir().join(format!("spa-rs-test-{}", rand_string(12)));
        let _cleanup = Cleanup(dir.clone());

        TestFiles.release(dir.clone()).unwrap();

        assert_eq!(
            fs::read_to_string(dir.join("assets/app.js")).unwrap(),
            "nested"
        );
    }

    #[test]
    fn concurrent_release_produces_complete_files() {
        let dir = temp_dir().join(format!("spa-rs-test-{}", rand_string(12)));
        let _cleanup = Cleanup(dir.clone());

        std::thread::scope(|s| {
            for _ in 0..8 {
                let dir = dir.clone();
                s.spawn(move || TestFiles.release(dir).unwrap());
            }
        });

        assert_eq!(
            fs::read_to_string(dir.join("index.html")).unwrap(),
            "test index"
        );
        assert_eq!(
            fs::read_to_string(dir.join("assets/app.js")).unwrap(),
            "nested"
        );
        assert_eq!(
            fs::read_to_string(dir.join(RELEASE_MARKER)).unwrap(),
            release_id::<TestFiles>()
        );
    }

    struct Cleanup(PathBuf);

    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[derive(rust_embed::RustEmbed)]
    #[folder = "tests/fixtures"]
    struct TestFiles;

    impl SpaStatic for TestFiles {}
}
