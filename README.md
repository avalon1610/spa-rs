# spa-rs

spa-rs is a library who can embed all SPA web application files (dist static file),
and release as a single binary executable.

It based-on [axum] and [rust_embed]

It reexported all axum module for convenient use.
## Example
```rust
use spa_rs::spa_server_root;
use spa_rs::SpaServer;
use spa_rs::routing::{get, Router};
use anyhow::Result;

spa_server_root!("web/dist");           // specific your SPA dist file location

#[tokio::main]
async fn main() -> Result<()> {
    let data = String::new();           // server context can be acccess by [axum::Extension]
    let mut srv = SpaServer::new()?
        .port(3000)
        .data(data)
        .static_path("/png", "web")     // static file generated in runtime
        .route("/api", Router::new()
            .route("/get", get(|| async { "get works" })
        )
    );
    srv.run(spa_server_root!()).await?;

    Ok(())
}
```

## Session
See [session] module for more detail.

## Dev
When writing SPA application, you may want use hot-reload functionallity provided
by SPA framework. such as [`vite dev`] or [`ng serve`].

You can use spa-rs to reverse proxy all static requests to SPA framework. (need enable `reverse-proxy` feature)

### Example
```rust
  let forward_addr = "http://localhost:1234";
  srv.reverse_proxy(forward_addr.parse()?);
```

License: MIT
