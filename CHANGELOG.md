# Changelog

All notable changes to the Lafiya smart contracts are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Each section must state whether the release changes the **storage schema**
(`SCHEMA_VERSION` bump) and, if so, what `migrate()` does — the production
upgrade runbook (`docs/runbooks/contract-upgrade.md`) checklists depend on it.

## [Unreleased]

### Added

- Contract upgradeability on both `attester-registry` and `attestation-registry`:
  admin-authorized `upgrade(new_wasm_hash)` replaces the contract's code with an
  already-uploaded wasm blob, leaving all storage untouched.
- Storage schema versioning: `SCHEMA_VERSION` constant (starts at `1`),
  `get_schema_version()` reader, and admin-authorized `migrate()` for pending
  schema migrations (including bootstrapping legacy pre-versioning instances from
  version `0`). See the storage versioning strategy in
  `docs/runbooks/contract-upgrade.md`.
- `docs/runbooks/contract-upgrade.md`: production runbook for safely performing an
  upgrade — pre-upgrade checklist, the `upgrade()` call sequence, wasm-hash
  verification against reviewed source, and handling of storage-schema-changing
  upgrades.
- `scripts/upgrade.sh`: automation for the mechanical upgrade steps (build,
  size-budget check, hash computation, upload with hash verification, `upgrade()`
  submission, optional `migrate()`, post-upgrade verification via
  `get_schema_version` and on-chain code fetch). Validated against a Stellar
  testnet deployment performing a real upgrade between two contract versions.
- This changelog.

### Schema impact

- **No storage-schema change.** Schema version `1` is layout-identical to the
  legacy (pre-versioning) layout; `DataKey::SchemaVersion` is appended without
  reordering existing variants. Upgrading a legacy deployment only requires a
  one-time `migrate()` to record version `1`.
