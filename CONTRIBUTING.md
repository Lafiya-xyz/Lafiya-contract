# Contributing to lafiya-contracts

This repo holds the Soroban smart contracts for Lafiya: an attester
allowlist and an attestation registry. See [README.md](README.md) for the
project overview and architecture.

## Prerequisites

- Rust (stable), installed via [rustup](https://rustup.rs)
- The `wasm32v1-none` target: `rustup target add wasm32v1-none`
- `pre-commit` (required for local git hooks): Install via `pip install pre-commit` or `brew install pre-commit`, then run `pre-commit install` in the repository root.

`rust-toolchain.toml` pins the toolchain and target automatically once
you run any `cargo` command in this repo.

## Workflow

```bash
make build   # cargo build --workspace
make test    # cargo test --workspace
make fmt     # cargo fmt --all
make clippy  # cargo clippy --workspace --all-targets -- -D warnings
make wasm    # release build for wasm32v1-none
make check   # fmt-check + clippy + test + wasm — run this before opening a PR
```

CI runs `make check`'s steps individually on every push and pull request;
a PR won't merge if any of them fail.

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
