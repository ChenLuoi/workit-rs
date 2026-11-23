default:
    @just --list

# Run this checkout in the .wk-dev workspace configured in .cargo/config.toml.
[positional-arguments]
wk *args:
    cargo run --locked -- "$@"

check:
    cargo fmt --check
    cargo clippy --locked --all-targets -- -D warnings
    cargo test --locked

# Pack and exercise the npm CLI locally using a temporary directory.
smoke-npm:
    cargo build --release --locked
    bash scripts/smoke-npm.sh
