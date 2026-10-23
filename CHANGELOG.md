# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.3.1] - 2026-10-23

### Fixed
- `axum-help`: `http_bail!` no longer emits a trailing semicolon in its expansion, so it can be used in expression position (e.g. a `match` arm) without triggering `semicolon_in_expressions_from_non_local_macros` (rust-lang/rust#79813)

## [0.10.0] - 2026-06-03

### Added
- `spa-rs`: `state()` method for providing axum state to `SpaServer`, enabling `State<T>` extractors in handlers
- `spa-rs`: `run_with_state()`, `run_tls_with_state()`, `run_api_with_state()`, `run_api_tls_with_state()` methods for `SpaServer<T>` with custom state
- `spa-rs`: `#[must_use]` attribute on `route_method()`

### Changed
- `spa-rs`: **BREAKING** `SpaServer::new()` now returns `Self` instead of `Result<Self>` — remove the `?` operator from call sites
- `spa-rs`: **BREAKING** `route()`, `route_method()`, `host_router()` now accept `Router<T>` / `MethodRouter<T>` instead of bare `Router` / `MethodRouter`, aligning with the server's state type
- `spa-rs`: **BREAKING** `run()`, `run_api()`, `run_tls()`, `run_api_tls()` are now only available on `SpaServer<()>` (the default). For custom state types, use `run_with_state()` etc.
- `spa-rs`: `data()` is deprecated in favor of `state()`
- `spa-rs`: `SpaServer` is now generic over `Router<T>` instead of `Router<()>`, with state conversion handled internally via `.with_state()`
- `spa-rs`: Edition upgraded to 2024
- `spa-rs`: Default release path changed from `current_exe` parent to `$HOME/.{pkg}_static_files`
- `spa-rs`: `extra_layer` closures now operate on `Router<()>` (post-state-conversion) instead of `Router`

### Fixed
- `spa-rs`: Fix compile errors caused by `Router<T>` vs `Router<()>` type mismatches in axum 0.8
- `spa-rs`: Fix `MatchHost` service not implementing `Service` trait due to unconstrained state type
- `spa-rs`: Fix doc example that incorrectly used `?` on `SpaServer::new()`

## [0.3.0] - 2026-06-03

### Changed
- `axum-help`: Edition upgraded to 2024
- `axum-help`: Bump `axum-extra` dev-dependency to 0.12.3
