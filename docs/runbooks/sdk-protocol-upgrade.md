# Runbook: Soroban SDK and Stellar Protocol Upgrades

Stellar protocol upgrades (new host functions, changed resource limits, new
ledger-entry types, changed fee models) and major `soroban-sdk` releases can
change **what compiles**, **the generated contract spec** (and so the
TypeScript bindings), **event encoding**, **test utilities**, and **resource
costs**. None of these changes show up unless someone looks for them. This
runbook is how we look.

The pinned versions are:

| What | Where |
|---|---|
| `soroban-sdk` | `[workspace.dependencies]` in `Cargo.toml` (source of truth; `README.md` is checked against it in CI by `.github/scripts/check_readme_versions.py`) |
| Rust toolchain | `rust-toolchain.toml` |
| `stellar-cli` | `STELLAR_CLI_VERSION` in `.github/workflows/compat-matrix.yml`, which is also the CLI to use locally for `make bindings` / `make conformance` |

## 1. Tracking upcoming changes

**Owner:** the contracts maintainers on rotation (see `.github/CODEOWNERS`).
The owner checks these at least monthly, and weekly in the month before a
protocol vote:

- **CAPs:** [stellar/stellar-protocol](https://github.com/stellar/stellar-protocol) — Core Advancement Proposals accepted for the next protocol.
- **SDF announcements:** the Stellar developer blog, the `#protocol-upgrades` / dev Discord channels, and the [software versions page](https://developers.stellar.org/docs/networks/software-versions), which gives each protocol's SDK/CLI/RPC versions and the testnet and mainnet vote dates.
- **SDK releases:** [stellar/rs-soroban-sdk releases](https://github.com/stellar/rs-soroban-sdk/releases) and its migration notes.
- **CLI releases:** [stellar/stellar-cli releases](https://github.com/stellar/stellar-cli/releases).
- **The weekly compatibility matrix** (below). A failure opens a
  "Weekly compatibility matrix failed" issue automatically, and that issue is
  often the earliest warning we get.

When a relevant change is announced, the owner opens a tracking issue labelled
`tooling` that links the CAP or release and records the vote dates.

## 2. Compatibility test matrix

`.github/workflows/compat-matrix.yml` runs every Monday (and on demand through
`workflow_dispatch`):

| Job | What it builds and tests against |
|---|---|
| `SDK (pinned)` | The exact `soroban-sdk` in `Cargo.lock`. This is the baseline, so a failure here means the toolchain or environment changed. |
| `SDK (latest-patch)` | The newest `soroban-sdk` release that satisfies `Cargo.toml`'s requirement (`cargo update -p soroban-sdk`). |
| `SDK (main)` | `rs-soroban-sdk`'s `main` branch, the next major release. Expect some failures here. Treat them as advance notice, not as blockers. |
| `Protocol (quickstart:latest)` | `make test-integration` on a local network running the **current** protocol. |
| `Protocol (quickstart:testing)` | `make test-integration` on a local network running the **next** protocol, ahead of the testnet vote. |

If any job fails on a scheduled run, the `report` job opens an issue, or
comments on the one that is already open.

## 3. Timing relative to network votes

| When | Action |
|---|---|
| New protocol lands in `quickstart:testing` | The matrix's `Protocol (quickstart:testing)` job starts exercising it. Fix any failures before the testnet vote. |
| SDK/CLI release for the new protocol published | Do the upgrade (section 4) on a branch. Merge it **before** the testnet vote if the release is backwards compatible with the current protocol; otherwise have it ready to merge the day testnet upgrades. |
| **Testnet** protocol vote | Redeploy / `upgrade()` the testnet contracts built with the new SDK and run `scripts/smoke-test.sh`. Leave it on testnet for at least one week. |
| **Mainnet** protocol vote (typically about 4–6 weeks after testnet) | Upgrade mainnet contracts only after the vote has passed and testnet has run cleanly. Follow [`contract-upgrade.md`](contract-upgrade.md). |

Never deploy a wasm that relies on a new host function or protocol feature to
a network that has not yet voted in that protocol. The upload fails, or
worse, the contract traps at runtime.

## 4. Pre-upgrade checklist

Work on a branch named `chore/soroban-sdk-<version>`. Tick each item in the PR
description.

- [ ] **Read the SDK changelog and migration guide** for every version skipped, and note breaking changes in the PR.
- [ ] **Bump the pins:** `soroban-sdk` in `Cargo.toml`, then `cargo update -p soroban-sdk`. Update `rust-toolchain.toml` if the SDK needs a newer Rust. Update `STELLAR_CLI_VERSION` if a new CLI is required.
- [ ] **Fix the README:** the `soroban-sdk` N.x mentions (CI fails otherwise).
- [ ] **Build and test:** `make check` and `make test-integration` against `quickstart:testing`.
- [ ] **Diff the contract spec:** `make conformance`. Any `check_snapshot.py` diff must be explained. An SDK-only bump should produce **no** spec changes. If it is intentional, `make conformance-update` and justify it per [`docs/stability-policy.md`](../stability-policy.md).
- [ ] **Diff the event encoding:** `gen_events_doc.py --check` (part of `make conformance`) must pass, or the `docs/events.md` diff must be explained. Indexers depend on it.
- [ ] **Re-run the budgets:** `make bench`, and compare against [`docs/storage-cost.md`](../storage-cost.md). Update the recorded numbers and investigate regressions over 10%.
- [ ] **Check wasm sizes** against the limit in `contract-upgrade.md`.
- [ ] **Re-run the upgrade-compatibility path:** upgrade a testnet deployment of the **previous release** to the new build with `scripts/upgrade.sh`, then confirm existing attestations and the allowlist still read correctly (`scripts/smoke-test.sh`).
- [ ] **Regenerate the bindings:** `make bindings`. `check_bindings_drift.py` must pass after you commit the result. Note any TypeScript type changes for `lafiya-web`.
- [ ] **Update `CHANGELOG.md`** with the SDK/protocol versions.

## 5. Rollback plan

- **Before merge:** close the branch. Nothing has shipped.
- **After merge, before deploy:** revert the bump commit. `Cargo.lock` pins the previous SDK exactly.
- **After a testnet/mainnet `upgrade()`:** contract upgrades are reversible,
  because the previous wasm stays on the ledger. Follow the rollback section
  of [`contract-upgrade.md`](contract-upgrade.md) to `upgrade()` back to the
  previous wasm hash recorded in the release manifest. This only works if the
  new build did **not** migrate the storage schema. SDK-only upgrades must not
  change `SCHEMA_VERSION`. If one does, it is not an SDK-only upgrade, and it
  needs the migration process in `contract-upgrade.md`.
- **A protocol change breaks a deployed contract:** the network cannot be
  rolled back. `pause()` the affected registries, ship a fix built with the
  new SDK through the normal upgrade process, and then `unpause()`.
