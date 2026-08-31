# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `attester-registry`: `add_attesters(Vec<Address>)` and
  `remove_attesters(Vec<Address>)` — admin-gated batch operations re-landing the
  feature from commit `5a93edd` that was silently dropped in the PR #94 merge
  (PROC-01 / issue #103). Both functions enforce `BATCH_LIMIT = 40` to stay
  within Soroban's per-transaction write-entry budget, skip already-present
  (add) or already-absent (remove) addresses idempotently, and emit one
  `AttesterAdded` / `AttesterRemoved` event per address actually changed.
  `Error::BatchTooLarge` (code `8`) is added and documented in
  `docs/error-codes.md`.
- `docs/adr/0006-attestation-revocation-semantics.md`: Decision section filled
  in and status moved to Accepted. Explicit choice: attestations are immutable
  historical records; responders independently check current attester status via
  `is_attester`; `revoke_attestation` is available for surgical per-record
  admin removal (ARCH-02 / issue #105).

### Fixed

- `attester-registry`: added missing `extend_ttl` calls to all state-mutating
  functions (`initialize`, `propose_admin`, `accept_admin`, `pause`, `unpause`,
  `remove_attester`, `set_max_attesters`, `suspend_attester`,
  `reinstate_attester`, `upgrade`, `migrate`) so that instance storage TTL is
  bumped on every write path, not only on `add_attester` /
  `add_attester_with_info` / `update_attester_info` (ARCH-01 / issue #104).
- `attestation-registry`: `attest()` now also extends the TTL of the specific
  `Attestation(record_hash, sequence)` persistent entry it writes, preventing
  archival of individual attestation records independently of instance storage
  (ARCH-01 / issue #104).

- ADR-0009 and a prototype release manifest: `scripts/generate_release_manifest.py`
  binds contract wasm hashes, storage schema versions, generated bindings, event
  schemas, and per-network deployment state into one JSON document
  (`docs/release-manifest/schema.json`), with `scripts/validate_release_manifest.py`
  and `scripts/check_manifest_compatibility.py` to validate it and let downstream
  repositories check compatibility before pinning a release. See
  `docs/adr/0009-release-manifest-and-compatibility.md`.
- GitHub issue templates: bug report, feature request, and a security
  report template that directs reporters to `SECURITY.md` instead of
  accepting inline disclosures.
- Pull request template with a checklist matching the expectations in
  `CONTRIBUTING.md` (`make check` passes, tests added or updated,
  `CHANGELOG.md` updated).
- `SECURITY.md` security policy with a private reporting channel.
- `attester-registry`: `update_attester_info`, an admin-authorized entry
  point for changing an already-allowlisted attester's metadata without
  re-enrolling it. Emits `AttesterInfoUpdated`, which is distinguishable
  from `AttesterAdded`, and fails with the new `Error::AttesterNotFound`
  when the attester is not currently allowlisted (never added, or since
  removed).
- `attester-registry`: `get_attester_status`, a combined read returning an
  attester's metadata together with its current suspension state in one
  call.
