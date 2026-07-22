---
title: "[audit] PROC-01: PR #94 merged without its own contract diff — batch-attester feature does not exist on main"
labels: bug, priority:p0, process, severity:high
---

**Severity:** High
**Difficulty:** N/A — not an exploit; a release-integrity defect already realized
**Type:** Software Supply Chain / Release Integrity

> Distinct from BUILD-01: not a defect in landed code, but a merged pull
> request whose actual diff never landed. Almost certainly enabled by
> CI-01 (no CI ran on the PR or on `main` post-merge to catch the
> mismatch).

## Summary

Commit `5a93edd`, on branch `issue-16-batch-attesters`, implements
`add_attesters`/`remove_attesters` with a budget bound and idempotency,
per its own commit message and diffstat. The merge commit that landed
this branch on `main`, `245d0f9` ("Merge pull request #94"), carries none
of the contract source changes — only incidental tooling/lockfile
changes. GitHub's UI shows PR #94 as merged; the feature it describes is
absent from `main`.

## Location

- Source commit (intended change): `5a93edd`
- Merge commit (actual landed change): `245d0f9`
- Absence confirmed at: `contracts/attester-registry/src/lib.rs`,
  `contracts/attester-registry/src/test.rs`, `docs/error-codes.md`

## Technical Detail

`5a93edd`'s diffstat:

```
 .cargo/config.toml                      |   5 +
 Cargo.lock                              | 268 ++++++++++++++++++--------------
 Cargo.toml                              |   2 +-
 contracts/attester-registry/src/lib.rs  |  65 +++++++-
 contracts/attester-registry/src/test.rs | 149 ++++++++++++++++++
 docs/error-codes.md                     |   1 +
 6 files changed, 374 insertions(+), 116 deletions(-)
```

`245d0f9`'s diffstat (the actual merge to `main`):

```
 .cargo/config.toml |   5 +
 Cargo.lock         | 268 ++++++++++++++++++++++++++++++-----------------------
 Cargo.toml         |   2 +-
 3 files changed, 160 insertions(+), 115 deletions(-)
```

`lib.rs`, `test.rs`, and `docs/error-codes.md` are absent from the merge
diff entirely.

## Proof of Concept

```
$ grep -rn batch contracts/ --include=*.rs -i
(no output)
```

No `add_attesters`, `remove_attesters`, `BATCH_LIMIT`, or
`Error::BatchTooLarge` exists anywhere in `contracts/` on `main`.

## Likely Root Cause

A merge-conflict resolution that took "ours" for the contract source
files while cleanly merging the non-conflicting, append-style tooling
files (`Cargo.lock`/`Cargo.toml`/`.cargo/config.toml`). Undetected because
CI-01 means `cargo test` never ran on the PR or on `main` post-merge — the
12 tests `5a93edd` claims to add never executed.

## Impact

- PR #94 and whatever issue it closed are misleading records: they claim
  a shipped feature that does not exist.
- Anyone integrating against the batch API (`lafiya-cli`, `lafiya-web`,
  or an operator following `docs/error-codes.md` if it had landed) is
  working against a feature that was never actually deployed.
- Demonstrates that a passing merge (green checkmark, or in this case no
  check at all per CI-01) does not guarantee the PR's stated diff landed —
  worth treating as a signal to spot-check other recent merges for the
  same failure mode, not assumed to be an isolated incident.

## Recommendation

1. Recover the dropped diff directly from `5a93edd` (`git show 5a93edd --
   contracts/ docs/error-codes.md`) and reapply it on current `main`,
   reconciled with BUILD-01's fixes (the original diff was authored
   against a differently-broken version of `lib.rs`).
2. Do not merge the reapplied change until CI-01 is fixed and the 12
   tests from `5a93edd` are observed passing in an actual CI run.
3. Spot-check other recently merged PRs (`git diff
   <merge-commit>^2..<merge-commit>` against the PR's own stated diffstat)
   for the same pattern, given it has now happened at least once
   undetected.

## Verification

- [ ] `add_attesters`/`remove_attesters` (or a deliberately reconsidered
      replacement) exist on `main`, admin-gated, budget-bounded,
      idempotent, matching the original specification or an explicit
      revision of it.
- [ ] Tests equivalent to `5a93edd`'s 12 tests pass in CI.
- [ ] `docs/error-codes.md` documents `BatchTooLarge`.
- [ ] `CHANGELOG.md` gets an `[Unreleased]` entry per `CONTRIBUTING.md`.
- [ ] At least one other recent merge is spot-checked for the same
      failure mode, with the result noted on this issue.
