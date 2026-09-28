.PHONY: build test fmt fmt-check clippy wasm wasm-contracts check clean \
        config-check config-list deploy upgrade smoke-test \
        bench conformance conformance-update preflight

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

# ---------------------------------------------------------------------------
# CLI-based operational targets (issues #401, #402)
# ---------------------------------------------------------------------------

# Run preflight checks against the selected network without any mutating operation.
# Usage: make preflight [NETWORK=testnet]
preflight:
	@echo "==> Running preflight checks for network: $(NETWORK)"
	lafiya-cli --network $(NETWORK) config show

# Show config for the selected network.
# Usage: make config-check [NETWORK=testnet]
config-check:
	lafiya-cli --network $(NETWORK) config show
	cargo test -p lafiya-config

# List all configured networks.
config-list:
	lafiya-cli config list

# Deploy contracts to the selected network via the Rust CLI.
# Usage: make deploy [NETWORK=testnet] [DRY_RUN=--dry-run] [SOURCE=--source alice]
DRY_RUN ?=
SOURCE  ?=
ADMIN   ?=

deploy:
	lafiya-cli --network $(NETWORK) deploy $(DRY_RUN) $(SOURCE) $(ADMIN)

# Upgrade a contract. Requires CONTRACT=attester-registry|attestation-registry.
# Usage: make upgrade CONTRACT=attester-registry [NETWORK=testnet] [SOURCE=--source alice]
CONTRACT ?= attester-registry

upgrade:
	lafiya-cli --network $(NETWORK) upgrade --contract $(CONTRACT) $(SOURCE) $(DRY_RUN)

# Run smoke tests against a deployed instance.
# Usage: make smoke-test [NETWORK=testnet]
smoke-test:
	lafiya-cli --network $(NETWORK) smoke-test $(DRY_RUN)
