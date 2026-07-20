# Contributing to Lafiya Smart Contracts

Thank you for your interest in contributing to Lafiya! Lafiya is an open-source Digital Public Good (DPG) aiming to bring verified, patient-controlled emergency health cards to the last mile. Your contributions help make this trust layer more robust, secure, and accessible.

This repository holds the Soroban (Stellar) smart contracts. Because Lafiya is a multi-repo ecosystem, contributions here often have ripple effects across other repositories. This guide outlines the setup, conventions, and workflows required to contribute safely.

- Rust (stable), installed via [rustup](https://rustup.rs)
- The `wasm32v1-none` target: `rustup target add wasm32v1-none`
- `pre-commit` (required for local git hooks): Install via `pip install pre-commit` or `brew install pre-commit`, then run `pre-commit install` in the repository root.

## Table of Contents
- [Local Setup](#local-setup)
- [Branching & Commit Conventions](#branching--commit-conventions)
- [Cross-Repo Coordination & Shared Contracts](#cross-repo-coordination--shared-contracts)
- [Database & Supabase Migrations](#database--supabase-migrations)
- [Smart Contract Development & Quality Standards](#smart-contract-development--quality-standards)
- [Pull Request Process](#pull-request-process)

---

## Local Setup

The core development environment requires Rust and the Soroban SDK.
For the quick-start commands, see the [Getting Started](README.md#getting-started) section in the `README.md`.

### Prerequisites
- **Rust (stable)**: Install via [rustup](https://rustup.rs).
- **Wasm target**: `rustup target add wasm32v1-none` (pinned via `rust-toolchain.toml`).
- **Stellar CLI**: Needed for deploying and interacting with the testnet. Install it via Cargo:
  ```bash
  cargo install --locked stellar-cli --features opt
  ```

---

## Branching & Commit Conventions

To maintain a clean, navigable history for auditability and open-source collaboration, we follow these conventions:

### Branch Naming
- `feature/short-description` for new features or smart contract functions.
- `bugfix/short-description` for bug fixes.
- `docs/short-description` for documentation-only changes.
- `chore/short-description` for build tasks, dependencies, etc.

### Commit Messages
We encourage [Conventional Commits](https://www.conventionalcommits.org/):
- `feat(registry): add batch attestation support`
- `fix(allowlist): correct signature verification check`
- `test(contracts): add tests for admin transfer`
- `docs: update contributing guide for cross-repo changes`

---

## Cross-Repo Coordination & Shared Contracts

Lafiya is composed of five distinct repositories in the `Lafiya-xyz` organization:
1. [lafiya-web](https://github.com/Lafiya-xyz/lafiya-web): Next.js web application (patient records, QR, allowlist management interface).
2. [lafiya-contracts](https://github.com/Lafiya-xyz/Lafiya-contract) (this repo): Soroban smart contracts (attester allowlist, attestation registry).
3. [lafiya-docs](https://github.com/Lafiya-xyz/lafiya-docs): Architectural documentation, threat model, and references.
4. [.github](https://github.com/Lafiya-xyz/.github): Organization-level files.
5. [lafiya-verifier](https://github.com/Lafiya-xyz/lafiya-verifier): Standalone verification tool.

### Shared Contracts Constraint
The on-chain attestation schema (a 32-byte record hash, attester Address, and timestamp) acts as a **shared contract** between `lafiya-contracts` and `lafiya-web`.
> [!IMPORTANT]
> If you modify a smart contract function signature, event payload, or the return shape of `get_attestation`, you **must** flag this change. It will break the off-chain patient profile and verification displays in `lafiya-web`.

**How to flag cross-repo changes:**
1. Check the **Cross-Repo Impact** section in the PR template.
2. Link the corresponding issue/PR in the `lafiya-web` repository.
3. Coordinate with maintainers to ensure both repositories are updated and deployed in tandem.

---

## Database & Supabase Migrations

While `lafiya-contracts` is a Rust smart contract repository and contains no database code:
- The main web application `lafiya-web` uses **Supabase** for its encrypted off-chain storage.
- If your contribution spans both the smart contracts and the database schema (e.g., adding field tracking for attestation IDs off-chain):

### Migration Guidelines (in `lafiya-web`)
1. **Supabase CLI**: Use the Supabase CLI to generate a new migration:
   ```bash
   supabase migration new your_migration_name
   ```
2. **Hand-Authored Types**: We use hand-authored types for database safety and strict runtime boundaries. The types are documented and maintained in:
   [lafiya-web/lib/supabase/types.ts](https://github.com/Lafiya-xyz/lafiya-web/blob/main/lib/supabase/types.ts)
   > [!WARNING]
   > Do **not** auto-generate database types and overwrite `lib/supabase/types.ts` blindly. Any schema change must have its typescript types updated by hand following the existing patterns to preserve custom wrappers, type guards, and safety boundaries.

---

## Smart Contract Development & Quality Standards

To maintain high security and minimize gas/storage costs on Soroban, all contract code must adhere to:

### Cross-Contract Calls
- Interact with other contracts (e.g., `attestation-registry` calling `attester-registry`) through a client trait interface using the `#[contractclient]` macro.
- Do not add direct crate dependencies between contracts to prevent linking duplicate symbols and bloating WASM binary sizes.

### Testing Requirements
- Every public contract function must have accompanying unit tests in its crate's `src/test.rs`.
- Tests must cover:
  - **Success paths**: Standard execution flow.
  - **Authorization paths**: Proper validation of admin or user signatures (`require_auth()`).
  - **Failure paths**: Rejection of double-initialization, invalid inputs, and unauthorized calls.
  - **Events**: Verify that correct events (like `AttesterAdded`, `AttestationRecorded`) are emitted.

### Local Quality Gate
Always run the validation suite locally before committing:
```bash
make check
```
This runs:
1. `make fmt` (code formatting verification)
2. `make clippy` (linter checks; warnings are treated as errors)
3. `make test` (all cargo tests)
4. `make wasm` (building target WASM binaries)

- Every new contract function needs unit tests covering both the success
  path and the failure/authorization paths (see `contracts/*/src/test.rs`
  for existing patterns using `soroban_sdk::testutils`).
- Cross-contract calls should go through a `#[contractclient]` trait
  interface (see `attestation-registry`'s `AttesterRegistryInterface`),
  not a direct crate dependency on the callee — depending on the whole
  crate links its contract implementation into your wasm build too.
- Any pull request (PR) that changes contract behavior, storage schemas, or public function signatures must include a corresponding entry in `CHANGELOG.md` under the `[Unreleased]` section. Refer to [releasing.md](docs/releasing.md) for details.
- Run `make check` locally before pushing; it's the same set of checks CI
  runs.
- Keep `Cargo.lock` committed and up to date so builds are reproducible.

## Pull Request Process

1. Fork the repository and create your branch from `main`.
2. Ensure your changes compile and pass all quality checks locally (`make check`).
3. Fill out the [Pull Request Template](.github/pull_request_template.md) completely, paying extra attention to the **Cross-Repo Impact** section if your changes touch shared interfaces.
4. An admin will review your PR. All checks in CI must pass before merging.
