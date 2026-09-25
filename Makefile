.PHONY: dev build test test-all clippy fmt check bench doc clean publish-dry

dev:
	cargo run --release

build:
	cargo build --release

test:
	cargo test --all-targets

test-doc:
	cargo test --doc

test-ignored:
	cargo test --release -- --ignored

clippy:
	cargo clippy --all-targets -- -D warnings

fmt:
	cargo fmt

fmt-check:
	cargo fmt --check

# All four gates must be clean before any change is considered done.
check: fmt-check clippy test test-doc

bench:
	cargo bench

doc:
	cargo doc --no-deps

publish-dry:
	cargo publish --dry-run

clean:
	cargo clean
