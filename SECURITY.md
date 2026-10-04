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
This relies on GitHub private vulnerability reporting being enabled for
the repository (Settings → Code security → Private vulnerability
reporting). Maintainers keep it enabled and check it as part of the
[triage runbook](docs/runbooks/security-triage.md#0-keep-the-intake-channel-open).
If the button is ever missing, open a public issue that says only "please
enable private vulnerability reporting" — with no details — and wait for
it to be turned on.

Include, where possible:

- A description of the vulnerability and its impact (e.g. forged
  attestation, allowlist bypass, auth-confusion in a cross-contract
  call).
- Steps to reproduce or a proof of concept (a failing test under
  `contracts/*/src/test.rs` is ideal).
- The contract(s) affected (`attester-registry`, `attestation-registry`,
  `multisig-account`), the network, the contract ID, and the commit.
- The severity you believe applies under the [rubric](#severity-rubric).
- Any suggested mitigation.

> Never include real personal or health data in a report. The contracts
> store only non-reversible hashes by design — keep reports the same
> way. See the privacy note in [README.md](README.md).

## What to expect

- **Acknowledgement** of your report within 3 business days.
- **Triage and severity assessment**, with follow-up questions routed
  back through the private advisory thread.
- **A fix and coordinated disclosure**: we aim to patch before any
  public detail is published, and we credit reporters (unless you'd
  rather stay anonymous).

## Threat model

The contract-layer STRIDE threat model, with attack trees mapped to
mitigations, is in [`docs/security/threat-model.md`](docs/security/threat-model.md).

## Bug bounty program

Lafiya runs a bug bounty program for the contracts and the verification
logic that decides whether a health record is "attested". The rules below
are the whole program; the operational side (SLAs, severity agreement,
publication) is in the
[security triage runbook](docs/runbooks/security-triage.md).

### Scope

In scope:

- The Soroban contracts in this repository — `contracts/attester-registry`,
  `contracts/attestation-registry`, `contracts/multisig-account`, and any
  contract added under `contracts/` later — **at the deployed contract IDs
  and commit recorded in the current release manifest** (see
  [docs/releasing.md](docs/releasing.md) and
  [`config/networks.toml`](config/networks.toml)). Reports against a
  commit on `main` that is not yet deployed are also accepted.
- The record commitment scheme (`crates/lafiya-commitment`, LRC-1; see
  [ADR 0008](docs/adr/0008-record-commitment-canonicalization.md)).
- The verifier SDK / CLI verdict logic: any path by which a client reports
  a record as attested, valid, or unrevoked when on-chain state says
  otherwise.
- Build/CI configuration in this repo where a defect could cause malicious
  artifacts to be trusted (release manifest, conformance gates).

Out of scope:

- The `lafiya-web` app and other sibling repositories (separate program);
  see the README's [Lafiya Organization](README.md#lafiya-organization)
  section.
- Vulnerabilities in the Stellar network, the Soroban SDK, third-party RPC
  providers, or the Rust toolchain — report those upstream.
- Social engineering, phishing, or physical attacks on maintainers or
  attesters.
- Denial of service through network spam or fee/resource exhaustion that
  any Stellar account could mount against any contract.
- Accepted risks already documented in an ADR (for example
  [ADR 0007](docs/adr/0007-unscoped-multisig-authorization.md)) or listed
  in the [audit known-issues register](audit/known-issues.md), unless the
  report shows impact beyond what the ADR accepts.

### Severity rubric

Severity follows Lafiya's impact model: the product is a trust layer, so
the worst outcome is a false "attested" verdict.

| Severity | Example impact |
|---|---|
| **Critical** | Make an arbitrary record verify as attested; take over admin or upgrade authority on a registry or the multisig; recover health data from on-chain data at scale. |
| **High** | Permanent verification outage (bricked registry, unrecoverable storage expiry); bypass attester suspension or attestation revocation; forge multisig authorization. |
| **Medium** | Griefing that removes or hides legitimate attestations; a temporary verification outage recoverable by an admin action. |
| **Low** | Event inconsistencies that mislead indexers; information leaks with limited impact; violations of documented invariants without a direct exploit. |

Final severity is agreed with the reporter using the process in the
triage runbook. Likelihood and required privileges can move a finding one
level down (for example, an issue that needs a compromised admin key).

### Rewards

Lafiya is a pre-revenue open-source project, so rewards are scaled to
what it can fund. Amounts are paid in USDC or XLM on Stellar, subject to
available funding at the time of the report.

| Severity | Reward (testnet phase) | Reward (after mainnet launch) |
|---|---|---|
| Critical | up to USD 2,000 | up to USD 10,000 |
| High | up to USD 750 | up to USD 3,000 |
| Medium | up to USD 250 | up to USD 750 |
| Low | Hall of fame + swag | up to USD 100 |

Every valid report, regardless of payout, earns public credit in the
GitHub Security Advisory and the [CHANGELOG](CHANGELOG.md) (unless you
prefer to stay anonymous) and a listing in the hall of fame below.

The reward pool is intended to be funded through Stellar ecosystem
programs — the Stellar Community Fund, the Stellar Development
Foundation's audit bank for ecosystem projects, and ecosystem contributor
programs such as Drips Wave — plus direct sponsorship. Only the first
report of a given issue is eligible; duplicates are credited, not paid.

### Rules of engagement

- Test **only on Stellar testnet or a local network** (`stellar network
  container start local`). Deploy your own instances of the contracts to
  test against; do not interfere with instances other people depend on.
- **Never exploit a vulnerability on mainnet**, and never access, modify,
  or destroy data that is not yours. Demonstrate impact with the minimum
  proof of concept needed.
- Never use real personal or health data (see below).
- Give us a reasonable time to fix the issue before disclosing it (see
  the timelines in the triage runbook; the default is 90 days).
- Do not demand payment as a condition of disclosure.

### Safe harbor

This safe-harbor language is adapted from the
[disclose.io](https://disclose.io) core terms.

When you research and report vulnerabilities in good faith and in line
with this policy, we consider your research authorized, and:

- We will not pursue or support legal action against you, including
  under anti-hacking laws (such as the CFAA) or anti-circumvention laws
  (such as the DMCA), for accidental, good-faith violations of this
  policy.
- If a third party brings legal action against you for activity that
  complied with this policy, we will make it known that your actions were
  authorized by us.
- We waive restrictions in our terms of use that would interfere with
  security research, for the purposes of that research only.

If you are unsure whether a planned action complies with this policy,
ask first through a private advisory. This safe harbor does not cover
mainnet exploitation, access to other people's data, or any activity that
harms Lafiya users.

### Hall of fame

No reports have been credited yet.

## Audits

| Date | Auditor | Scope | Report |
|---|---|---|---|
| — | — | — | No external audit has been completed yet. |
| Planned (before mainnet) | To be selected | Contracts in [audit/scope.md](audit/scope.md) | Will be linked here |

The auditor onboarding package lives in [`audit/`](audit/README.md).
[`config/networks.toml`](config/networks.toml) marks mainnet deployment
as requiring an audit.

## Supported versions

The project is pre-release (`0.x`). Only the latest commit on `main` is
supported with security fixes; there are no maintained release branches
yet.

## After a fix

Once a vulnerability is fixed, details may be published via a GitHub
security advisory and noted in [CHANGELOG.md](CHANGELOG.md). General
contributions (non-security) follow the workflow in
[CONTRIBUTING.md](CONTRIBUTING.md).

## CI supply-chain hardening

The CI pipeline builds the wasm that becomes trusted contract code, so it is
part of the trust base.

- **Pinned actions.** Every `uses:` reference is pinned to a full commit SHA
  with the release in a trailing comment (`uses: actions/checkout@<sha> # v7.0.1`).
  Dependabot's `github-actions` ecosystem (`.github/dependabot.yml`) opens PRs
  that bump the SHA and the comment together. Never reference an action by tag
  or branch.
- **Least-privilege tokens.** Every workflow declares top-level
  `permissions: contents: read`. Jobs raise it only where needed:
  `docs.yml` (`contents: write`, to push `gh-pages`), `stale.yml`
  (`issues`/`pull-requests: write`), and `scorecard.yml` / `security-scan.yml`
  (`security-events: write`, plus `id-token: write` for Scorecard publishing).
- **Verified downloads.** Binaries fetched with `curl` (stellar-cli in
  `smoke-test.yml`, gitleaks in `security-scan.yml`) are checked against a
  pinned SHA-256 with `sha256sum -c`. Bump the URL and checksum together.
- **Runner hardening.** The release (`release-manifest.yml`) and deploy
  (`docs.yml`) jobs run `step-security/harden-runner` in `egress-policy: audit`.
  After reviewing the observed endpoints from a few runs, switch to
  `egress-policy: block` with an `allowed-endpoints` list.
- **OpenSSF Scorecard** (`scorecard.yml`) runs weekly and on every push to
  `main`, uploads results to code scanning, and publishes the README badge.
  Baseline score: *not yet published* (no Scorecard run existed before this
  workflow). Record the first run's score here as the "before" value and the
  score after the fixes it reports as the "after" value.

### Recommended ruleset for `main`

Maintainers should apply these via **Settings → Rules → Rulesets** (they
cannot be set from a pull request):

| Setting | Value |
| --- | --- |
| Restrict deletions / block force pushes | On |
| Require a pull request before merging | On, 1 approval |
| Require review from Code Owners | On (see `.github/CODEOWNERS`) |
| Dismiss stale approvals on new commits | On |
| Require status checks to pass | `Shell lint (smoke-test.sh)`, `Format`, `Clippy`, `Test`, `Build (wasm32v1-none)`, `docs`, `CodeQL (*)`, `Semgrep (Soroban rules)`, `gitleaks` |
| Require branches to be up to date before merging | On |
| Require signed commits | Optional (recommended once all maintainers sign) |
| Bypass list | Empty (admins included) |

**Would this have blocked the `lafiya-cli` build breakage?** Only if the
failing job is a *required* check and branches must be up to date. A
breakage that passes on a stale PR branch but fails after merging with a
newer `main` is exactly what "require branches to be up to date" catches;
without required checks, a red CI run does not prevent merging. Enabling
both settings above closes that gap.
