.PHONY: build test fmt fmt-check clippy wasm test-integration check clean

build:
	cargo build --workspace

test:
	cargo test --workspace

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

wasm:
	cargo build --workspace --release --target wasm32v1-none

test-integration: wasm
	./tests/integration/run.sh

check: fmt-check clippy test wasm

clean:
	cargo clean

