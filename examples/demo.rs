use anyhow::Result;
use axum::Router;
use spa_rs::{routing::get, spa_server_root, SpaServer};

spa_server_root!("web/dist");

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    SpaServer::new()
        .data(123)
        .port(3001)
        .static_path("/png", "web")
        .route(
            "/api",
            Router::new().route("/get", get(|| async { "get works" })),
        )
        .run(spa_server_root!())
        .await?;

    Ok(())
}