---
name: Bug report
about: Report a reproducible problem with the contracts, build, or tests.
title: "[bug]: "
labels: bug
---

> **Security vulnerabilities:** do **not** report them here. Follow the
> private disclosure process in [SECURITY.md](SECURITY.md) instead.
>
> **Privacy:** never paste real personal or health data (names, record
> contents, QR payloads) into an issue — only synthetic test data and
> non-reversible hashes. See the privacy note in the
> [README](README.md#privacy--compliance).

## Description

A clear, concise description of the bug.

## Steps to reproduce

1.
2.
3.

## Expected behavior

What you expected to happen.

## Actual behavior

What actually happened. If a contract call failed with an error code,
include the code and the contract that produced it — codes are
contract-scoped and listed in [docs/error-codes.md](docs/error-codes.md).

## Environment

- Contract(s) affected: <!-- attester-registry / attestation-registry / build tooling -->
- `rustc --version`:
- `soroban-sdk` version (from `Cargo.toml` / `Cargo.lock`):
- OS / platform:

## Logs and output

Paste the relevant output of the failing command — `make check` runs
`fmt` + `clippy` + `test` + the wasm build, the same steps CI runs (see
[CONTRIBUTING.md](CONTRIBUTING.md)).

```text

```

## Additional context

Anything else: links, screenshots, a minimal repro branch, etc.
