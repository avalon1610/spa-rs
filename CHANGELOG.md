# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.10.2] - 2026-10-27

### Fixed
- `spa-rs`: Default release directory is now derived from the executable name and a hash of the embedded file set (`$HOME/.{exe}_static_files_{hash}`) instead of a random suffix. Restarting the same binary reuses its directory, different applications never share one, and a rebuild with changed assets gets a fresh directory
- `spa-rs`: Extracted files are written to a temporary file and renamed into place, so a concurrently running instance never serves a partially written file
- `spa-rs`: A release marker records which file set a directory holds; when it matches and every file is present, extraction is skipped. Directories left incomplete by an interrupted run are repaired on the next start
- `spa-rs`: Leftover `.spa-rs-*.tmp` files from an interrupted extraction are removed, but only once they are an hour old, so a concurrent process's in-flight temporary file is never deleted
- `spa-rs`: Without `$HOME`, the release directory is placed in the system temp dir with a random suffix, since a predictable name in the shared temp dir could be pre-created by another local user

### Changed
- `spa-rs`: `SpaServer::release_path()` now overrides the derived default (`release_path` is stored as `Option<PathBuf>`); the derived path is only computed when it is unset

## [0.10.1] - 2026-10-27

### Fixed
- `spa-rs`: Default release directory now gets a random suffix (`$HOME/.{pkg}_static_files_{random}`) so multiple concurrently running instances no longer write over each other's extracted static files
- `spa-rs`: `rand_string` is now shared between the digest auth module and the release path instead of being duplicated

### Known limitations
- `spa-rs`: release directories are never cleaned up, so each run leaves a new `$HOME/.{pkg}_static_files_{random}` directory behind; set `release_path()` to a fixed path if accumulation matters (documented on `SpaServer::release_path()`)

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
