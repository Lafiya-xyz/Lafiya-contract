# Security Policy

Lafiya's contracts are **pre-alpha, unaudited, and targeted at Stellar
testnet** — but they anchor a health-adjacent trust layer, so
vulnerabilities matter even before mainnet. Please report them
responsibly.

## Reporting a vulnerability

**Do not open a public GitHub issue, PR, or discussion for a security
report.** Public disclosure before a fix exists puts users at risk.

Instead, report privately via GitHub's private vulnerability reporting:
open this repository's **Security** tab → **Advisories** →
[**Report a vulnerability**](../../security/advisories/new).

Include, where possible:

- A description of the vulnerability and its impact (e.g. forged
  attestation, allowlist bypass, auth-confusion in a cross-contract
  call).
- Steps to reproduce or a proof of concept (a failing test under
  `contracts/*/src/test.rs` is ideal).
- The contract(s) affected: `attester-registry` and/or
  `attestation-registry`.
- Any suggested mitigation.

> Never include real personal or health data in a report. The contracts
> store only non-reversible hashes by design — keep reports the same
> way. See the privacy note in [README.md](README.md).

## What to expect

- **Acknowledgement** of your report within a few days.
- **Triage and severity assessment**, with follow-up questions routed
  back through the private advisory thread.
- **A fix and coordinated disclosure**: we aim to patch before any
  public detail is published, and we credit reporters (unless you'd
  rather stay anonymous).

## Scope

In scope:

- The Soroban contracts in this repository (`contracts/attester-registry`,
  `contracts/attestation-registry`) — authorization, initialization,
  storage, events, and the cross-contract call between them.
- Build/CI configuration in this repo where a defect could cause
  malicious artifacts to be trusted.

Out of scope:

- Vulnerabilities in the Stellar network, the Soroban SDK, or Rust
  toolchain themselves — report those upstream.
- The `lafiya-web` app and other sibling repositories (they have, or
  will have, their own policies); see the README's
  [Lafiya Organization](README.md#lafiya-organization) section.

## Supported versions

The project is pre-release (`0.x`). Only the latest commit on `main` is
supported with security fixes; there are no maintained release branches
yet.

## After a fix

Once a vulnerability is fixed, details may be published via a GitHub
security advisory and noted in [CHANGELOG.md](CHANGELOG.md). General
contributions (non-security) follow the workflow in
[CONTRIBUTING.md](CONTRIBUTING.md).
