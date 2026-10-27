# Runbook: Triaging a Security Report

**Audience:** maintainers on security rotation who receive a private
vulnerability report for this repository.

**Scope:** every report that arrives through GitHub private vulnerability
reporting (or is moved there from another channel), from intake to public
advisory. The researcher-facing rules — scope, severity rubric, rewards,
and safe harbor — are in [SECURITY.md](../../SECURITY.md#bug-bounty-program).
This runbook covers what maintainers do with a report.

> ⚠️ Never discuss an unfixed vulnerability in a public issue, PR, commit
> message, Discord channel, or CI log. All work happens in the advisory
> thread and its temporary private fork until the advisory is published.

---

## 0. Keep the intake channel open

Private vulnerability reporting must stay enabled, or researchers have no
private channel:

1. Repository **Settings → Code security → Private vulnerability
   reporting → Enable** (requires admin).
2. Confirm from a non-maintainer account that **Security → Report a
   vulnerability** is visible.
3. Check it after any change to repository settings or ownership, and at
   least once per quarter:

   ```bash
   gh api repos/Lafiya-xyz/Lafiya-contract/private-vulnerability-reporting
   # expected: {"enabled":true}
   ```

4. Make sure at least two maintainers **watch** the repository with
   security alerts enabled, so a report is never seen by one person only.

## 1. Response SLAs

Clocks start when the advisory is created. "Business days" are Monday to
Friday.

| Step | Critical | High | Medium | Low |
|---|---|---|---|---|
| Acknowledge the reporter | 1 business day | 3 business days | 3 business days | 5 business days |
| Initial severity assessment | 3 business days | 5 business days | 10 business days | 10 business days |
| Fix merged (target) | 7 days | 30 days | 60 days | 90 days |
| Public disclosure (default) | After the fix is deployed, at most 90 days after the report | | | |

If an SLA is going to slip, tell the reporter before it does, with a new
date.

## 2. Intake

1. Acknowledge the report in the advisory thread. Thank the reporter,
   link to SECURITY.md, and give the date by which they will hear the
   initial assessment.
2. Assign an **owner** (drives the report to closure) and a **second
   reviewer** (checks the owner's severity call and fix).
3. Check for duplicates among open advisories and the
   [audit known-issues register](../../audit/known-issues.md). If it is a
   duplicate or a documented accepted risk, say so, credit the reporter,
   and close as described in [step 7](#7-closing-without-a-fix).
4. If the report contains real personal or health data, delete it from
   the thread, tell the reporter, and continue with synthetic data only.

## 3. Reproduce

1. Reproduce on a **local network or testnet only**, against the commit
   named in the report and against `main`.
2. Write the reproduction as a failing test (`contracts/*/src/test.rs`
   for contract bugs). Keep it in the advisory's temporary private fork,
   not in a public branch.
3. Identify every deployed contract ID that runs affected code, using the
   release manifest and [`config/networks.toml`](../../config/networks.toml).

## 4. Agree on severity

1. The owner rates the finding with the rubric in
   [SECURITY.md](../../SECURITY.md#severity-rubric) and records the reasoning
   in the thread: impact, required privileges, likelihood, affected
   deployments.
2. The second reviewer confirms or challenges the rating.
3. Share the rating with the reporter. If they disagree, each side states
   the concrete impact scenario it relies on. Maintainers decide within
   five business days of the disagreement. When the two positions stay
   one level apart, use the higher severity.
4. Set the severity field on the advisory, and request a CVE from the
   advisory page for Medium and above.

## 5. Fix

1. Develop the fix in the advisory's **temporary private fork** (advisory
   page → "Start a temporary private fork"). Include the reproduction
   test.
2. The fix needs review from a maintainer other than its author. For
   contract changes, also run `make check` and `make conformance`.
3. For deployed contracts, follow
   [contract-upgrade.md](contract-upgrade.md). Critical and High fixes
   can use the pause switch (`pause()`) on the registries while the
   upgrade is prepared; record when it was paused and unpaused.
4. Record the fixed commit and the new wasm hashes in the release manifest.

## 6. Disclose

1. Agree the publication date with the reporter. Default: once the fix is
   deployed to every affected network, and no later than 90 days after the
   report. Maintainers may extend this once, by at most 30 days, if a
   deployment is blocked; tell the reporter why.
2. Complete the advisory: affected versions and commits, patched commit,
   CVSS or rubric severity, CWE, description, workaround, and credit to
   the reporter (unless they asked to stay anonymous).
3. Merge the private fork and **publish the GitHub Security Advisory**.
   GitHub publishes the CVE, if one was assigned.
4. Add an entry to [CHANGELOG.md](../../CHANGELOG.md) under **Security** that
   links the advisory, and add the reporter to the hall of fame in
   SECURITY.md.
5. Pay the reward as described in
   [SECURITY.md](../../SECURITY.md#rewards) and record the payment transaction
   in the advisory thread.

## 7. Closing without a fix

Close the advisory without publication when the report is:

- **out of scope** or a **documented accepted risk**: explain which rule
  or ADR applies, and invite the reporter to show impact beyond it;
- **not reproducible**: after at least one request for more details
  and 14 days without a reply;
- a **duplicate**: credit the reporter, and link them to the original
  advisory once it is published.

Even when you close without a fix, record the outcome, severity (if any),
and reasoning in the thread.

## 8. Post-incident follow-up

Within two weeks of publishing a Critical or High advisory:

- Add the violated property to the invariant list in
  [`audit/invariants.md`](../../audit/invariants.md), with the regression test
  that now checks it.
- Open (public, post-disclosure) issues for any process or tooling gaps
  found during triage.

## 9. Indexer image CVE and signature alerts

**Audience:** whoever is on call when the indexer security alerts fire.
These alerts are defined as code in the indexer `PrometheusRule`
(`deploy/observability/indexer-rules.yaml`) and are documented alongside
the SLOs in [`docs/observability.md`](../observability.md). Each alert's
`runbook_url` annotation points at this section.

Alert names and what they mean:

| Alert | Meaning |
|---|---|
| `LafiyaIndexerImageScanFailed` | The image scan gate (Trivy/Grype) failed or did not produce a report for the currently deployed indexer image. |
| `LafiyaIndexerImageCriticalCVE` | The deployed indexer image contains a Critical or High CVE that is not covered by an approved exception. |
| `LafiyaIndexerImageSignatureInvalid` | `cosign verify` failed for the deployed indexer image: missing signature, wrong key/identity, or digest mismatch. |
| `LafiyaIndexerScanExceptionStale` | An entry in the scan exception file is past its expiry or no longer matches a finding. |

### 9.1 Signature verification failure

Treat a signature failure as a supply-chain incident until proven
otherwise. Do **not** restart or roll the deployment to "clear" it.

1. Identify the running image digest:
   `kubectl -n lafiya get deploy lafiya-indexer -o jsonpath='{.spec.template.spec.containers[0].image}'`.
2. Re-run verification locally against the pinned public key/identity:
   `cosign verify --key cosign.pub <image>@<digest>`.
3. If verification fails, confirm the digest matches the release manifest
   and the CI build that produced it. A digest that does not match any
   signed build is a compromise signal: follow
   [security-triage.md](security-triage.md) (private advisory, no public
   discussion) and roll back to the last verified digest.
4. If verification passes locally but the alert persists, the alert's
   key/identity configuration is stale; fix the rule and note it in the
   alert's issue.

### 9.2 Scan gate failure or unapproved Critical/High CVE

1. Pull the scan report for the deployed digest from the CI artifact or
   re-run the scanner: `trivy image <image>@<digest>`.
2. If the CVE has a fixed base image or dependency version, bump it and
   ship through the normal release process.
3. If no fix is available, either accept the risk through the exception
   file (see 9.3) or roll back to the last image without the finding.
4. A scan gate that failed to *produce* a report is a tooling failure:
   re-run the gate, and if it keeps failing, open an issue and treat the
   image as unscanned (do not promote it).

### 9.3 Exception-file review

Exceptions live in `deploy/observability/scan-exceptions.yaml`. Every entry
must have an `id` (CVE or finding), an `owner`, a `justification`, and an
`expires` date no more than 90 days out.

1. When `LafiyaIndexerScanExceptionStale` fires, open the exception file
   and remove or renew the entry.
2. Renew only with a written justification and a named owner; never renew
   silently.
3. Review the whole file at least monthly and after every Critical/High
   advisory, and record the review date in the file header.
