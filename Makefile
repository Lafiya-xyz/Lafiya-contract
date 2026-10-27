.PHONY: build build-indexer build-qr test test-indexer test-qr fmt fmt-check clippy lint deny wasm wasm-contracts check clean config-check config-list deploy bench conformance conformance-update audit-env

# Crates that are workspace members but not part of the contract wasm build.
INDEXER_CRATE ?= lafiya-indexer
QR_CRATE ?= lafiya-qr

# Compatibility shim: these targets delegate to `cargo xtask` (CONTRIBUTING.md)
# and will be removed after the next release.
build:
	cargo build --workspace

# Aggregate build: workspace plus the indexer and QR crates (all features).
build-indexer:
	cargo build -p $(INDEXER_CRATE) --all-targets --all-features

build-qr:
	cargo build -p $(QR_CRATE) --all-targets --all-features

test:
	cargo xtask test

# Indexer e2e tests need a Postgres service (see .github/workflows/ci.yml).
test-indexer:
	cargo test -p $(INDEXER_CRATE) --all-features

test-qr:
	cargo test -p $(QR_CRATE) --all-features

fmt:
	cargo xtask fmt

fmt-check:
	cargo xtask fmt --check

clippy:
	cargo xtask clippy

# Lint aggregate: workspace clippy plus the indexer and QR crates, warnings denied.
lint: clippy
	cargo clippy -p $(INDEXER_CRATE) --all-targets --all-features -- -D warnings
	cargo clippy -p $(QR_CRATE) --all-targets --all-features -- -D warnings

# Dependency/license/source policy check (deny.toml).
deny:
	cargo deny check

wasm:
	cargo build -p attester-registry -p attestation-registry -p multisig-account --release --target wasm32v1-none

# Builds only the Soroban contract crates for wasm32v1-none. Unlike `wasm`,
# this doesn't try (and fail) to cross-compile the std-only workspace
# members (lafiya-cli, lafiya-config, lafiya-commitment) for a no_std-only
# target -- see the matching comment in .github/workflows/ci.yml.
wasm-contracts:
	cargo xtask wasm

# Bit-for-bit reproducible contract wasm (docs/releasing.md#reproducible-builds):
# `cargo xtask wasm --reproducible` inside a Rust image pinned by digest, with
# the checkout mounted at a fixed path. Must match rust-toolchain.toml.
REPRODUCIBLE_IMAGE ?= rust:1.98.1-slim-trixie@sha256:f47a8de237dcbb0b0ce1099901e60a89728e3d51f24e664b40e947171538ade7

wasm-reproducible:
	docker run --rm -v "$(CURDIR)":/lafiya -w /lafiya \
		-e LAFIYA_GIT_COMMIT=$$(git rev-parse HEAD) -e SOURCE_DATE_EPOCH=$$(git log -1 --format=%ct) \
		$(REPRODUCIBLE_IMAGE) sh -c 'rustup target add wasm32v1-none && cargo xtask wasm --reproducible; \
			s=$$?; chown -R $(shell id -u):$(shell id -g) target; exit $$s'
	sha256sum target/wasm32v1-none/release/*.wasm | tee target/wasm32v1-none/release/SHA256SUMS

test-integration: wasm-contracts
	./tests/integration/run.sh

check: fmt-check lint test test-indexer test-qr wasm-contracts deny

bindings: wasm
	stellar contract bindings typescript --wasm target/wasm32v1-none/release/attester_registry.wasm --output-dir bindings/attester-registry --overwrite
	stellar contract bindings typescript --wasm target/wasm32v1-none/release/attestation_registry.wasm --output-dir bindings/attestation-registry --overwrite
	stellar contract bindings typescript --wasm target/wasm32v1-none/release/multisig_account.wasm --output-dir bindings/multisig-account --overwrite

conformance: wasm-contracts
	python3 scripts/conformance/check_snapshot.py
	python3 scripts/conformance/check_error_docs.py
	python3 scripts/conformance/check_web_error_mapping.py
	python3 scripts/conformance/gen_events_doc.py --check
	python3 scripts/conformance/gen_catalog.py --check
	python3 scripts/conformance/check_bindings_drift.py

conformance-update: wasm-contracts
	python3 scripts/conformance/check_snapshot.py --update
	python3 scripts/conformance/gen_events_doc.py
	python3 scripts/conformance/gen_catalog.py

clean:
	cargo clean

audit-env:
	./scripts/audit-env.sh $(if $(DEPLOY),--deploy)

bench:
	cargo xtask budgets

NETWORK ?= testnet

config-check:
	./scripts/admin.sh --network $(NETWORK) config show
	cargo test -p lafiya-config

config-schema:
	cargo run --quiet -p lafiya-cli -- config schema > config/networks.schema.json

stellar-toml:
	python3 scripts/generate_stellar_toml.py

config-list:
	./scripts/admin.sh --network $(NETWORK) config list

deploy:
	./scripts/deploy.sh --network $(NETWORK)
