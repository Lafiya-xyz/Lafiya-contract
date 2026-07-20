# Release Process

This document outlines the versioning, changelog, and release workflow for the Lafiya smart contracts.

---

## Versioning Policy

Lafiya smart contracts follow [Semantic Versioning (SemVer)](https://semver.org/).
- **Major (`X.0.0`)**: Released for backward-incompatible changes, such as breaking public API signature updates, storage schema changes that require state migration, or major architectural shifts.
- **Minor (`0.Y.0`)**: Released for backward-compatible features, such as adding new optional functions, events, or helper modules.
- **Patch (`0.0.Z`)**: Released for backward-compatible bug fixes, internal optimizations, or documentation updates.

### When to Cut a Version
A new version should be cut whenever:
1. A milestone (e.g., M1 Attestation, M2 Incentives) is reached and unit tests pass.
2. A feature branch containing public API changes or storage schema adjustments is merged into `main`.
3. An audit remediation or critical fix is ready for testnet/mainnet.

---

## Release Workflow

### 1. Changelog Updates
- **PR Author's Responsibility**: Anyone opening a Pull Request that modifies contract behavior, public interfaces, or storage schemas **must** add a description of their changes to the `[Unreleased]` section of the [CHANGELOG.md](../CHANGELOG.md) file.
- **Reviewer's Responsibility**: Reviewers must verify that the PR author has updated the changelog before approving the PR.

### 2. Version Bump & Tagging
When a release is ready to be finalized:
1. Update the version in the workspace root [Cargo.toml](../Cargo.toml) under `[workspace.package]`.
2. Move the entries in [CHANGELOG.md](../CHANGELOG.md) from the `[Unreleased]` section into a new version block with the current date.
3. Commit these changes:
   ```bash
   git add Cargo.toml CHANGELOG.md
   git commit -m "chore: release vX.Y.Z"
   ```
4. Create a git tag pointing to the release commit:
   ```bash
   git tag -a vX.Y.Z -m "Release vX.Y.Z"
   git push origin main --tags
   ```

---

## Testnet & Mainnet Redeployment

Because Soroban smart contracts are immutable once deployed (unless an upgrade path is explicitly programmed), deploying a new version generally requires deploying new WASM bytecode and updating the contract addresses referenced by downstream consumers (such as the frontend app `lafiya-web`).

For details on the redeployment, initialization, and upgrade state migration processes, please cross-reference the **upgrade-runbook issue** on GitHub:
- [Upgrade Runbook Issue #48 (GitHub)](https://github.com/Lafiya-xyz/Lafiya-contract/issues/48)

Always follow the instructions in the runbook when performing redeployments to ensure that downstream services are not interrupted.
