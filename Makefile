.PHONY: build test fmt fmt-check clippy wasm wasm-contracts check clean config-check config-list deploy bench conformance conformance-update

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

# Builds only the Soroban contract crates for wasm32v1-none. Unlike `wasm`,
# this doesn't try (and fail) to cross-compile the std-only workspace
# members (lafiya-cli, lafiya-config, lafiya-commitment) for a no_std-only
# target -- see the matching comment in .github/workflows/ci.yml.
wasm-contracts:
	cargo build --release --locked --target wasm32v1-none -p multisig-account -p attester-registry -p attestation-registry

test-integration: wasm
	./tests/integration/run.sh

check: fmt-check clippy test wasm

bindings: wasm
	stellar contract bindings typescript --wasm target/wasm32v1-none/release/attester_registry.wasm --output-dir bindings/attester-registry --overwrite
	stellar contract bindings typescript --wasm target/wasm32v1-none/release/attestation_registry.wasm --output-dir bindings/attestation-registry --overwrite

conformance: wasm-contracts
	python3 scripts/conformance/check_snapshot.py
	python3 scripts/conformance/check_error_docs.py
	python3 scripts/conformance/gen_events_doc.py --check
	python3 scripts/conformance/check_bindings_drift.py

conformance-update: wasm-contracts
	python3 scripts/conformance/check_snapshot.py --update
	python3 scripts/conformance/gen_events_doc.py

clean:
	cargo clean

bench:
	cargo test -p attester-registry large_attester_allowlist_load -- --nocapture

NETWORK ?= testnet

config-check:
	./scripts/admin.sh --network $(NETWORK) config show
	cargo test -p lafiya-config

config-list:
	./scripts/admin.sh --network $(NETWORK) config list

deploy:
	./scripts/deploy.sh --network $(NETWORK)
