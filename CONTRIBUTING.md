# Contributing to Lafiya Smart Contracts

Thank you for your interest in contributing to Lafiya! Lafiya is an open-source Digital Public Good (DPG) aiming to bring verified, patient-controlled emergency health cards to the last mile. Your contributions help make this trust layer more robust, secure, and accessible.

This repository holds the Soroban (Stellar) smart contracts. Because Lafiya is a multi-repo ecosystem, contributions here often have ripple effects across other repositories. This guide outlines the setup, conventions, and workflows required to contribute safely.

See [Local Setup > Prerequisites](#prerequisites) for the tools you'll need before contributing.

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

### Recommended: devcontainer / Codespaces

The recommended setup is the devcontainer in [`.devcontainer/`](.devcontainer/).
Open the repository in VS Code with the Dev Containers extension ("Reopen in
Container") or create a GitHub Codespace. It provides, with every version
pinned and every download checksum-verified:

- the Rust channel from `rust-toolchain.toml` with `wasm32v1-none`, `rustfmt`,
  `clippy`, plus nightly for `cargo-fuzz`;
- `stellar-cli`, `cargo-nextest`, `cargo-mutants`, `cargo-fuzz`, `just`;
- Python 3.12 with `jsonschema`, and Node 22 LTS with pnpm;
- sidecars: `stellar/quickstart --local` (Soroban RPC on port 8000, with a
  health check) and `postgres` for the indexer.

On creation it builds the contract wasm, adds the `local` network to
`stellar-cli`, and creates and funds a `dev` identity. After that, `make check`
and `make test-integration` work with no manual steps.

**Codespaces prebuilds:** maintainers enable them under *Settings → Codespaces
→ Prebuild configurations* for `main`, using `.devcontainer/devcontainer.json`.
The image build and `onCreateCommand` (wasm build) run in the prebuild, so a
new codespace is ready in about two minutes. The `Devcontainer` workflow
builds the container and runs `make check` inside it so the setup can't rot.

If you would rather set up your machine by hand, install the prerequisites below.

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

### Fuzz / Property-Based Testing
`attester-registry` and `attestation-registry` each have a `fuzz_test.rs` module (built with [`proptest`](https://docs.rs/proptest), already a dev-dependency in both crates) alongside their regular `test.rs`:
- `attester-registry::fuzz_test` generates arbitrary sequences of `add_attester`/`remove_attester`/`suspend_attester`/`reinstate_attester` calls over a small pool of addresses and checks, after every step, that `is_attester` agrees with a plain-Rust model — i.e. it never observes an address as simultaneously allowlisted and not.
- `attestation-registry::fuzz_test` calls `attest` with arbitrary/adversarial 32-byte `record_hash` values (including all-zero and all-`0xFF`) and in unusual orderings relative to `initialize`, asserting it only ever returns a typed `Result` and never panics.

CI runs these with a small, time-bounded case count (`PROPTEST_CASES=256`, see `.github/workflows/ci.yml`) as a **non-blocking** job — a regression there is a signal to investigate, not a merge blocker, since proptest's case count/seed is inherently variable run to run.

To fuzz much harder locally (uncapped, until you stop it or hit a shrink-and-report), raise the case count and optionally the max shrink iterations:
```bash
PROPTEST_CASES=100000 cargo test -p attester-registry fuzz_test -- --nocapture
PROPTEST_CASES=100000 cargo test -p attestation-registry fuzz_test -- --nocapture
```
If proptest finds a failing case, it shrinks it to a minimal repro and writes it to `contracts/<crate>/proptest-regressions/fuzz_test.txt`; commit that file alongside your fix so the minimal input becomes a permanent regression test (proptest replays entries from that file automatically on every run).

If a crash or invariant violation surfaces while working on either fuzz target, file it as its own bug report and fix it — don't fold an unrelated fix into a feature PR.

### Task Runner
Developer and CI tasks run through `cargo xtask` (the `xtask/` workspace member, aliased
in `.cargo/config.toml`). It needs only the Rust toolchain pinned in `rust-toolchain.toml`
(installed automatically by rustup), so it works the same on Linux, macOS, and Windows —
no GNU make or WSL required. Run `cargo xtask help` for the full list:

| Task | What it does |
|------|--------------|
| `cargo xtask check` | fmt `--check`, clippy (`-D warnings`), tests, contract wasm, conformance |
| `cargo xtask ci` | `check` plus `test --all-features` — exactly what CI runs on all three OSes |
| `cargo xtask fmt [--check]` / `clippy` / `test [--all-features]` / `fuzz` | individual steps |
| `cargo xtask wasm [--reproducible]` | contract wasm; `--reproducible` as used for releases (docs/releasing.md) |
| `cargo xtask bindings` | regenerate TypeScript bindings (needs `stellar-cli`) |
| `cargo xtask conformance [--update]` | interface snapshot, error docs, events doc, bindings drift |
| `cargo xtask budgets` | large-allowlist resource-budget load test |
| `cargo xtask release-manifest` / `docs` | release manifest; rustdoc with `-D warnings` |

The conformance scripts are still Python (stdlib only, plus `stellar-cli` to decode the contract
spec); `check` skips them with a warning when `stellar` isn't on `PATH`, `ci` in a full
environment runs them. Porting them to Rust (decoding the spec with `stellar-xdr`, with
equivalence tests against the Python output on the snapshot fixtures) is tracked as follow-up work
to #427; until then `xtask` is the only entry point, so the port won't change how they're invoked.

The `Makefile` remains for one release as a thin compatibility shim (`make check` →
`cargo xtask check`) and will then be removed.

### Local Quality Gate
Always run the validation suite locally before committing:
```bash
cargo xtask check
```

There is no `rustfmt.toml` in this repo — that's intentional, not an oversight. `cargo xtask fmt --check` runs against rustfmt's default settings, and PRs should not introduce a custom formatting config.


- Every new contract function needs unit tests covering both the success
  path and the failure/authorization paths (see `contracts/*/src/test.rs`
  for existing patterns using `soroban_sdk::testutils`).
- Cross-contract calls should go through a `#[contractclient]` trait
  interface (see `attestation-registry`'s `AttesterRegistryInterface`),
  not a direct crate dependency on the callee — depending on the whole
  crate links its contract implementation into your wasm build too.
- Use [Conventional Commits](https://www.conventionalcommits.org/) for PR titles and
  squash-merge messages: the CHANGELOG and version bump are generated from them
  (see [releasing.md](docs/releasing.md)). A commit that changes `DataKey` or any
  `#[contracttype]` must carry a `Schema-Impact:` trailer, or the release PR fails.
- Run `cargo xtask check` locally before pushing; CI runs `cargo xtask ci`.
- Keep `Cargo.lock` committed and up to date so builds are reproducible.

### On-chain Data Review Checklist

Anything written to the ledger is public and permanent. Before adding or
changing a `#[contracttype]` stored under a `DataKey`, or any
`#[contractevent]`, answer these in the PR description:

- [ ] **Personal data?** Can the value, alone or combined with other on-chain
      data, timing, or a small cohort (e.g. one LGA), identify a CHW or a
      patient? If yes or maybe, justify why it must be on-chain.
- [ ] **No plaintext or low-entropy health data.** Any hash over patient or
      CHW attributes includes a mandatory random salt (≥128 bits) kept
      off-chain, so it can be crypto-shredded.
- [ ] **No reasons or free text** about a person (e.g. suspension reason codes).
- [ ] **Minimised granularity.** Use the coarsest region/time precision that
      works; prefer storage reads over events for data that doesn't need to be
      indexed.
- [ ] **Erasure path.** You can say what an NDPA erasure request means for
      this element (shred salt, revoke, remove, or accepted risk).
- [ ] **Inventory updated.** [docs/compliance/ndpa-onchain-analysis.md](docs/compliance/ndpa-onchain-analysis.md)
      section 3 (and section 5 if risk changes) reflects the new field.

## Pull Request Process

1. Fork the repository and create your branch from `main`.
2. Ensure your changes compile and pass all quality checks locally (`cargo xtask check`).
3. Fill out the [Pull Request Template](.github/pull_request_template.md) completely, paying extra attention to the **Cross-Repo Impact** section if your changes touch shared interfaces.
   If your change alters a public surface (contract functions, storage, events, error codes, bindings, CLI, or the commitment scheme), follow the [Interface Stability and Deprecation Policy](docs/stability-policy.md) and answer **Breaking change?** in the template.
4. An admin will review your PR. All checks in CI must pass before merging.
