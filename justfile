default:
    @just --list

# Run this checkout in the .wk-dev workspace configured in .cargo/config.toml.
[positional-arguments]
wk *args:
    cargo run --locked -- "$@"

# Delete the local development workspace, including all its clones and tasks.
clean-dev:
    rm -rf -- .wk-dev

check:
    cargo fmt --check
    cargo clippy --locked --all-targets -- -D warnings
    cargo test --locked

# Exercise workspace, project, task and tool selection prompts in a temporary workspace.
smoke-interactive:
    cargo build --locked
    node scripts/smoke-interactive.mjs target/debug/wk

# Pack and exercise the npm CLI locally using a temporary directory.
smoke-npm:
    cargo build --release --locked
    bash scripts/smoke-npm.sh
