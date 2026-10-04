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
