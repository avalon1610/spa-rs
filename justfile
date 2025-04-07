check:
    cargo fmt
    cargo hack clippy --each-feature --exclude-all-features  -- -D warnings
    cargo test
    cargo machete --with-metadata