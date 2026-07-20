.PHONY: build test fmt fmt-check clippy wasm check clean config-check config-list deploy

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

check: fmt-check clippy test wasm

clean:
	cargo clean

bench:
	cargo test -- --nocapture
