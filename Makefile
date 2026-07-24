.PHONY: build test fmt fmt-check clippy wasm check clean config-check config-list deploy bench

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

bindings: wasm
	stellar contract bindings typescript --wasm target/wasm32v1-none/release/attester_registry.wasm --output-dir bindings/attester-registry --overwrite
	stellar contract bindings typescript --wasm target/wasm32v1-none/release/attestation_registry.wasm --output-dir bindings/attestation-registry --overwrite

clean:
	cargo clean

bench:
	cargo test -p attester-registry large_attester_allowlist_load -- --nocapture
