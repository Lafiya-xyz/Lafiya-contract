# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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
- `attester-registry`: `set_max_attesters` and `get_max_attesters`, an
  admin-configurable soft cap on the number of allowlisted attesters
  (defaulting to 50,000). `add_attester`/`add_attester_with_info` fail with
  the new `Error::AllowlistFull` when the allowlist is at capacity and the
  attester is not already present; lowering the cap never evicts existing
  attesters, it only blocks further additions.
- `attester-registry`: `suspend_attester` and `reinstate_attester`, admin-
  authorized entry points for temporarily blocking an allowlisted attester
  from attesting without removing it. Suspended attesters fail
  `is_attester` until reinstated; emits `AttesterSuspended` and
  `AttesterReinstated` respectively.
