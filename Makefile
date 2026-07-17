.PHONY: build test fmt fmt-check clippy wasm check clean deploy-testnet init-contracts bindings

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

deploy-testnet:
	bash scripts/deploy-testnet.sh

init-contracts:
	bash scripts/init-contracts.sh

bindings:
	bash scripts/bindings.sh
