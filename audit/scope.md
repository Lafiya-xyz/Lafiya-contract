# Audit Scope

## Commit

The audited commit is the commit hash recorded when the change freeze
starts (see [change-freeze.md](change-freeze.md)). Until then, scope is
described against `main`. Record the final value here:

| Field | Value |
|---|---|
| Repository | https://github.com/Lafiya-xyz/Lafiya-contract |
| Freeze commit | _to be filled at freeze_ |
| Final audited commit | _to be filled at audit close_ |
| Toolchain | Rust `stable` (see `rust-toolchain.toml`), target `wasm32v1-none`, `soroban-sdk` 27.0.0 |

## In scope

The on-chain contracts. Test files (`test.rs`, `*_test.rs`) are **not**
in scope for findings, but auditors should read them as specifications.

| Crate | Path | Code lines¹ | Total lines |
|---|---|---|---|
| `attester-registry` | `contracts/attester-registry/src/lib.rs` | 507 | 744 |
| `attestation-registry` | `contracts/attestation-registry/src/lib.rs` | 339 | 498 |
| `multisig-account` | `contracts/multisig-account/src/lib.rs` | 116 | 181 |
| **Total** | | **962** | **1,423** |

¹ Non-blank, non-comment lines, as of the scoping commit. Regenerate at
freeze with:

```bash
tokei contracts/*/src/lib.rs          # or: cloc contracts/*/src/lib.rs
```

Also in scope, as configuration:

- `Cargo.toml` release profile (`overflow-checks = true`, `panic = "abort"`).
- `Cargo.lock` (dependency pinning of `soroban-sdk`).

## Optional (separate engagement or best-effort)

Off-chain components that affect whether a verdict is trusted, but that do
not hold or move on-chain state:

| Component | Path | Why it matters |
|---|---|---|
| Record commitment (LRC-1) | `crates/lafiya-commitment` | Canonicalization bugs could make two records share a commitment ([ADR 0008](../docs/adr/0008-record-commitment-canonicalization.md)) |
| Admin CLI | `crates/lafiya-cli` | Builds admin transactions |
| RPC resilience | `crates/lafiya-rpc-resilience` | Transaction retry and recovery ([ADR 0011](../docs/adr/0011-rpc-provider-failover-and-transaction-recovery.md)) |
| Release manifest and conformance tooling | `scripts/*.py`, `scripts/conformance/` | Decide whether a release is "compatible" |
| Deploy and upgrade scripts | `scripts/*.sh` | Operator-run upgrade flow |

## Out of scope

- `lafiya-web` and other sibling repositories.
- The Stellar network, Soroban host, `soroban-sdk`, and RPC providers.
- Generated TypeScript bindings in `bindings/` (generated from the Wasm;
  drift is checked by `make conformance`).
- Documentation, except where it states a security property.
