use anyhow::Result;
use axum::{
    http::HeaderValue,
    routing::{get_service, Router},
};
#[cfg(debug_assertions)]
use http::Method;
use http::{header, StatusCode};
use log::{debug, error, warn};
use std::{
    env::temp_dir,
    fs::{self, create_dir_all},
    net::SocketAddr,
    path::{Path, PathBuf},
};
use tower::ServiceExt;
#[cfg(debug_assertions)]
use tower_http::cors::{Any, CorsLayer};
use tower_http::{services::ServeDir, set_header::SetResponseHeaderLayer};

pub use axum::*;
pub use rust_embed::RustEmbed;

pub use misc::http_bail;
pub use misc::http_err;
pub use misc::http::*;

pub struct SpaServer<T> {
    static_path: Option<(String, PathBuf)>,
    data: Option<T>,
    port: u16,
    routes: Vec<(String, Router)>,
}

impl<T> SpaServer<T>
where
    T: Clone + Sync + Send + 'static,
{
    pub fn new() -> Self {
        Self {
            static_path: None,
            data: None,
            port: 8080,
            routes: Vec::new(),
        }
    }

    pub async fn run<Root>(self, root: Root) -> Result<()>
    where
        Root: SpaStatic,
    {
        let embeded_path = root.release()?;

        #[cfg(debug_assertions)]
        let cors = CorsLayer::new()
            .allow_methods([Method::GET, Method::POST])
            .allow_headers(Any)
            .allow_origin(Any);

        let mut app = Router::new().fallback(
            get_service(ServeDir::new(&embeded_path))
                .layer(Self::add_cache_control())
                .handle_error(|e| async move {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!(
                            "Unhandled internal server error {:?} when serve embeded path {}",
                            e,
                            embeded_path.display()
                        ),
                    )
                })
                .map_response(|mut response| {
                    if response.status() == StatusCode::NOT_FOUND {
                        response
                            .headers_mut()
                            .insert(header::LOCATION, HeaderValue::from_static("/"));
                        *response.status_mut() = StatusCode::TEMPORARY_REDIRECT;
                    }

                    response
                }),
        );

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

        #[cfg(debug_assertions)]
        {
            app = app.layer(cors)
        }

        if let Some(data) = self.data {
            app = app.layer(Extension(data));
        }

        Server::bind(&format!("0.0.0.0:{}", self.port).parse()?)
            .serve(app.into_make_service_with_connect_info::<SocketAddr>())
            .await?;

        Ok(())
    }

    pub fn route(mut self, path: impl Into<String>, router: Router) -> Self {
        self.routes.push((path.into(), router));
        self
    }

    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    pub fn data(mut self, data: T) -> Self {
        self.data = Some(data);
        self
    }

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

pub trait SpaStatic: RustEmbed {
    fn release(&self) -> Result<PathBuf> {
        let target_dir = temp_dir().join(format!("{}_static_files", env!("CARGO_PKG_NAME")));
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
