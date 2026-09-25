default:
    @just --list

format:
    cargo fmt --all

lint:
    cargo fmt --all -- --check
    cargo clippy --locked --all-targets -- -D warnings

test:
    cargo test --locked

docs:
    RUSTDOCFLAGS='-D warnings' cargo doc --locked --no-deps

check: lint test docs

build:
    cargo build --locked --release
