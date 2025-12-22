use anyhow::Result;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Router, extract::Request, routing::get};
use headers::{Authorization, HeaderMapExt, authorization::Basic};
use spa_rs::{SpaServer, help::filter::FilterExLayer, spa_server_root};

spa_server_root!("web/dist");

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let srv = SpaServer::<()>::new()?
        .port(3001)
        .static_path("/png", "web")
        .route(
            "/api",
            Router::new()
                .route("/get", get(|| async { "get works" }))
                .layer(FilterExLayer::new(|request: Request<_>| {
                    if let Some(_auth) = request.headers().typed_get::<Authorization<Basic>>() {
                        // TODO: do something
                        Ok(request)
                    } else {
                        Err(StatusCode::UNAUTHORIZED.into_response())
                    }
                })),
        );
    srv.run(spa_server_root!()).await?;

    Ok(())
}
