.PHONY: dev build test test-doc test-ignored clippy fmt check bench doc clean publish-dry verify-crash-safe

# All cargo invocations go through scripts/safe-cargo.sh, which imposes a hard
# cgroup memory cap. A 15 GB host cannot survive an uncapped release build of
# this dependency tree; see scripts/safe-cargo.sh for the incident.
CARGO := ./scripts/safe-cargo.sh

dev:
	$(CARGO) run --release

build:
	$(CARGO) build --release

test:
	$(CARGO) test --all-targets

test-doc:
	$(CARGO) test --doc

test-ignored:
	$(CARGO) test --release -- --ignored

clippy:
	$(CARGO) clippy --all-targets -- -D warnings

fmt:
	cargo fmt

fmt-check:
	cargo fmt --check

# All four gates must be clean before any change is considered done.
check: fmt-check clippy test test-doc

bench:
	$(CARGO) bench

doc:
	$(CARGO) doc --no-deps

publish-dry:
	$(CARGO) publish --dry-run

# Proves the memory bound that the OOM crash fix claims. 8 MB error bodies at
# 300 RPS for 15s. Peak RSS must stay in the tens of MB, not the tens of GB.
verify-crash-safe:
	@echo "Requires a target serving large error bodies. See AGENTS.md section 4.1."
	./target/release/rustress -u http://127.0.0.1:8080/big -r 300 -d 15 --timeout 20

clean:
	cargo clean
