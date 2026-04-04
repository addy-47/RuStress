.PHONY: dev test lint build clean

dev:
	cargo run

test:
	cargo test --workspace

lint:
	cargo fmt -- --check
	cargo clippy --workspace -- -D warnings

build:
	cargo build --release

clean:
	cargo clean
