# Security Policy

## Scope

This policy covers the Soroban smart contracts in this repository — the
attester allowlist and attestation registry that make up Lafiya's on-chain
trust layer.

It does **not** cover the Lafiya backend/frontend ([`lafiya-web`](https://github.com/Lafiya-xyz/Lafiya-web))
or the offline/QR verification client ([`lafiya-verifier`](https://github.com/Lafiya-xyz/Lafiya-verifier)).
If you've found a vulnerability in one of those, please report it against
that repository instead — each repo maintains its own security policy and
disclosure channel.

## Supported Versions

This project is **pre-alpha**, deployed to Stellar **testnet only**, and has
not yet been audited (see the [Disclaimer](README.md#disclaimer) in the
README). There is no stable release branch yet — only `main` is supported.
Once a mainnet deployment and versioned releases exist, this section will be
updated to reflect which versions receive security fixes.

## Reporting a Vulnerability

**Do not open a public GitHub issue for a security vulnerability.** Public
issues are for bugs and feature requests; a vulnerability report made public
before a fix is available puts every user of the deployed contracts at risk.

Instead, report privately using GitHub's built-in mechanism:

1. Go to the [Security tab](https://github.com/Lafiya-xyz/Lafiya-contract/security) of this repository.
2. Click **"Report a vulnerability"** to open a private advisory.
3. Include as much detail as you can: the affected contract/function, the
   conditions required to trigger the issue, and — if possible — a proof of
   concept (a failing test case is ideal, since this repo's contracts are
   fully covered by `soroban_sdk::testutils`-based unit tests).

If you're unable to use GitHub's private reporting for any reason, contact a
maintainer directly through their GitHub profile rather than filing a public
issue.

### What to expect

- **Acknowledgement:** within 3 business days of your report.
- **Initial assessment:** within 7 days, including whether the report is
  confirmed, its severity, and a rough timeline for a fix.
- **Disclosure:** once a fix is merged and (for on-chain issues) deployed, we
  will coordinate with you on public disclosure timing. We ask that you not
  disclose the issue publicly until a fix is available.

Given the project's pre-alpha, testnet-only status, response times may be
faster than the above for critical issues (e.g. anything that could let an
unauthorized party register a fraudulent attestation) — but please don't
assume a slower response means the report isn't taken seriously.

## Why This Matters Here

The attestation registry is the cryptographic backbone of Lafiya's trust
model: it's what lets a first responder trust that a patient's emergency
health record was verified by a real, allowlisted health worker. A
vulnerability that lets an unauthorized address register itself as an
attester, or lets an attester's authorization be bypassed, undermines that
trust model entirely — even though no health data itself is ever stored
on-chain.
