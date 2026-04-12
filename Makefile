.PHONY: all build test fmt lint clippy

all: fmt lint test build

build:
	cargo build

test:
	cargo test

fmt:
	cargo fmt
	taplo fmt Cargo.toml taplo.toml deny.toml

lint: clippy

clippy:
	cargo clippy --all-targets -- -D warnings
