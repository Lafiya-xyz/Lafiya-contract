# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Offline validation of operator input across the admin tooling. Stellar
  addresses, contract IDs, 32-byte record hashes, network names, RPC URLs and
  `--source`/`--admin` values are checked before the `stellar` CLI is invoked.
  The Rust implementation (`lafiya_config::validation`) verifies the full strkey
  CRC16 checksum; `scripts/lib/validate.sh` provides the same structural checks
  for `deploy.sh` and `admin.sh` and ships an offline `--self-test`.
- Partially deployed network profiles (only one registry ID recorded in
  `config/networks.toml`) are now reported explicitly by both the Rust CLI and
  the shell scripts.

- GitHub issue templates: bug report, feature request, and a security
  report template that directs reporters to `SECURITY.md` instead of
  accepting inline disclosures.
- Pull request template with a checklist matching the expectations in
  `CONTRIBUTING.md` (`make check` passes, tests added or updated,
  `CHANGELOG.md` updated).
- `SECURITY.md` security policy with a private reporting channel.

### Changed

- `deploy.sh` and `lafiya-cli deploy` refuse to run without a transaction source
  and an admin address unless `--dry-run` or `--build-only` is used.
