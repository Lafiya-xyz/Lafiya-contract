# Lafiya External Audit Package

This directory is the onboarding package for external security auditors
(and for anyone reviewing the contracts with fresh eyes). It aims to get
an auditor from `git clone` to a working build, a mental model of the
system, and a list of the properties that matter within one day, so the
audit time goes to finding real risk.

Read it in this order:

| # | Document | What it answers |
|---|---|---|
| 1 | [scope.md](scope.md) | Which commit, crates, and files are in scope, and how big they are |
| 2 | [system-overview.md](system-overview.md) | Architecture, trust assumptions, admin topology, accepted risks |
| 3 | [invariants.md](invariants.md) | Numbered properties the contracts must uphold, mapped to the tests that check them |
| 4 | [known-issues.md](known-issues.md) | Open and accepted security-relevant issues, so they are not re-reported |
| 5 | [questions.md](questions.md) | Specific questions we want the auditors to answer |
| 6 | [environment.md](environment.md) | One-command reproducible build, test, fuzz, and local deploy |
| 7 | [change-freeze.md](change-freeze.md) | How fixes during the audit are tracked and the audited commit recorded |
| 8 | [dry-run-feedback.md](dry-run-feedback.md) | Feedback from contributors who dry-ran this package |

Security reporting rules, the bounty program, and the list of past and
planned audits are in [SECURITY.md](../SECURITY.md).

## Keeping the package current

Update this package in the same PR as any change that:

- adds, removes, or renames a public contract function, error, or event;
- changes an admin or authorization path;
- closes or opens an item in [known-issues.md](known-issues.md);
- adds a test that checks an invariant (add it to the mapping in
  [invariants.md](invariants.md)).

Before an audit starts, a maintainer who did not write the latest changes
reviews the whole package (the "fresh eyes" check) and records their
sign-off in [change-freeze.md](change-freeze.md#sign-off).
