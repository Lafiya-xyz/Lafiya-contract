# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Changelog file (`CHANGELOG.md`) to track contract changes.
- Release process documentation (`docs/releasing.md`).

### Changed
- Updated `CONTRIBUTING.md` to require a changelog entry in PRs that change contract behavior.

## [0.1.0] - 2026-07-17

### Added
- Initial smart contract implementation for **`attester-registry`**:
  - Storage schema and custom error types.
  - Core functions: `initialize`, `add_attester`, `remove_attester`, and `is_attester`.
  - Administrative authorization checks and emission of `AttesterAdded` and `AttesterRemoved` events.
  - Unit tests covering initialization, double-initialization, allowlist administration, lookups, and event emission.
- Initial smart contract implementation for **`attestation-registry`**:
  - Storage schema, `Attestation` type, and custom error types.
  - Core functions: `initialize`, `attest`, and `get_attestation`.
  - Authorization checks, cross-contract allowlist validation via `attester-registry`, and emission of `AttestationRecorded` events.
  - Unit tests covering initialization, attestations, lookup logic, and error scenarios.
- Comprehensive codebase tooling and CI:
  - Makefile with targets for `build`, `test`, `fmt`, `clippy`, `wasm`, and `check`.
  - GitHub Actions workflow running tests, format verification, and clippy lints on push/PR.
  - Workspace configuration pinning Rust toolchain, target `wasm32v1-none`, and `soroban-sdk` 25.3.1.
- Initial repository documentation and project metadata:
  - Scoped README outlining project overview, smart contract architecture, and setup instructions.
  - `CONTRIBUTING.md` documenting local developer workflow and guidelines.
  - MIT LICENSE setup.
  - `docs/error-codes.md` documenting all error codes across contracts.

### Changed
- Optimized `attestation-registry` to call `attester-registry` via a local `#[contractclient]` trait interface instead of a direct crate dependency, preventing redundant contract code linking and WASM build errors.
- Enforced strict safety lints by denying `unwrap`, `expect`, and `panic` in registry contracts.

[Unreleased]: https://github.com/Lafiya-xyz/Lafiya-contract/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Lafiya-xyz/Lafiya-contract/releases/tag/v0.1.0
