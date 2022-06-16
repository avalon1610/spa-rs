use anyhow::Result;
use http::{Request, StatusCode};
use spa_rs::{
    filter::FilterExLayer,
    headers::{authorization::Basic, Authorization, HeaderMapExt},
    response::IntoResponse,
    routing::get,
    routing::Router,
    spa_server_root, SpaServer,
};

spa_server_root!("web/dist");

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let mut srv = SpaServer::<()>::new();
    srv.port(3001).static_path("/png", "web").route(
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
