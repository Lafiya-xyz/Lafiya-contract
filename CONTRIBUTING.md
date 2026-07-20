# Contributing to lafiya-contracts

This repo holds the Soroban smart contracts for Lafiya: an attester
allowlist and an attestation registry. See [README.md](README.md) for the
project overview and architecture.

## Prerequisites

- Rust (stable), installed via [rustup](https://rustup.rs)
- The `wasm32v1-none` target: `rustup target add wasm32v1-none`
- `stellar-cli` (Stellar CLI): install via `cargo install --locked stellar-cli` or `curl -sSL https://raw.githubusercontent.com/stellar/stellar-cli/main/install.sh | sh`
- Docker (for local Soroban network integration testing)

`rust-toolchain.toml` pins the toolchain and target automatically once
you run any `cargo` command in this repo.

## Workflow

```bash
make build             # cargo build --workspace
make test              # cargo test --workspace
make fmt               # cargo fmt --all
make clippy            # cargo clippy --workspace --all-targets -- -D warnings
make wasm              # release build for wasm32v1-none
make test-integration  # runs end-to-end integration tests against a local Soroban node
make check             # fmt-check + clippy + test + wasm — run this before opening a PR
```

CI runs `make check`'s steps and `make test-integration` on every push and pull request;
a PR won't merge if any of them fail.

## Local Integration Testing

To run the integration test suite against a local Soroban network locally:

1. **Start the local Soroban container** (Quickstart sandbox):
   ```bash
   docker run -d -p 8000:8000 --name stellar stellar/quickstart:testing --local
   ```
2. **Run the integration test suite**:
   ```bash
   make test-integration
   ```

The test runner script (`tests/integration/run.sh`) will automatically check local network health, configure the `local` network profile in `stellar-cli`, generate and fund test identities via Friendbot, deploy both contracts, initialize them, test non-allowlisted attester rejection, allowlist an attester, and verify end-to-end attestation recording and lookup.

## Guidelines


- Every new contract function needs unit tests covering both the success
  path and the failure/authorization paths (see `contracts/*/src/test.rs`
  for existing patterns using `soroban_sdk::testutils`).
- Cross-contract calls should go through a `#[contractclient]` trait
  interface (see `attestation-registry`'s `AttesterRegistryInterface`),
  not a direct crate dependency on the callee — depending on the whole
  crate links its contract implementation into your wasm build too.
- Run `make check` locally before pushing; it's the same set of checks CI
  runs.
- Keep `Cargo.lock` committed and up to date so builds are reproducible.

## Reporting issues

Open a [GitHub issue](https://github.com/Lafiya-xyz/Lafiya-contract/issues).
