---
title: "[bug]: PR #94 (batch add/remove attesters) merged without its own contract code — feature does not exist on main"
labels: bug, priority:p0, process
---

> Distinct from the general "main doesn't compile" issue (#02): this isn't
> a code defect in a landed feature, it's a **merged PR whose feature
> never actually landed**. Likely enabled by #01 (CI never ran, so nothing
> flagged that the merge diff didn't match the PR's stated diff).

## Description

Commit `5a93edd` — *"feat(attester-registry): add batch add/remove
attesters with idempotency and budget bound"* — on branch
`issue-16-batch-attesters`, per its own commit message and diffstat, adds:

- `add_attesters(Vec<Address>)` / `remove_attesters(Vec<Address>)`
- `BATCH_LIMIT` constant and `Error::BatchTooLarge`
- 12 new unit tests
- a `docs/error-codes.md` entry for `BatchTooLarge`
- plus unrelated tooling changes (`Cargo.toml`, `Cargo.lock`,
  `.cargo/config.toml`)

```
 .cargo/config.toml                      |   5 +
 Cargo.lock                              | 268 ++++++++++++++++++--------------
 Cargo.toml                              |   2 +-
 contracts/attester-registry/src/lib.rs  |  65 +++++++-
 contracts/attester-registry/src/test.rs | 149 ++++++++++++++++++
 docs/error-codes.md                     |   1 +
 6 files changed, 374 insertions(+), 116 deletions(-)
```

But the merge commit on `main`, `245d0f9` — *"Merge pull request #94 from
Agencybuilds/issue-16-batch-attesters"* — only carries:

```
 .cargo/config.toml |   5 +
 Cargo.lock         | 268 ++++++++++++++++++++++++++++++-----------------------
 Cargo.toml         |   2 +-
 3 files changed, 160 insertions(+), 115 deletions(-)
```

`contracts/attester-registry/src/lib.rs`, `src/test.rs`, and
`docs/error-codes.md` are **absent** from the merge. Confirmed on the
current tree:

```
$ grep -rn batch contracts/ --include=*.rs -i
(no output)
```

There is no `add_attesters`, `remove_attesters`, `BATCH_LIMIT`, or
`Error::BatchTooLarge` anywhere in `contracts/`. GitHub shows PR #94 as
merged, closing whatever issue tracked "issue-16-batch-attesters," but the
actual contract change it describes does not exist on `main`. Anyone
relying on the PR history (changelog, issue tracker, `lafiya-cli` or
`lafiya-web` integration work planned against the batch API) will be
working against a feature that was never actually shipped.

## How this likely happened

Almost certainly a merge-conflict resolution that took "ours" for the
contract source files while correctly merging the non-conflicting tooling
files (`Cargo.lock`/`Cargo.toml`/`.cargo/config.toml`, which are
append/version-bump style changes that merge cleanly). Nothing caught it
because (see #01) the CI workflow that would run `cargo test` on the PR
and on `main` post-merge cannot even parse, so the 12 new tests this PR
claims to add never ran, on the PR or after.

## Expected behavior

Merging PR #94 should have landed `add_attesters`, `remove_attesters`,
`BATCH_LIMIT`, `Error::BatchTooLarge`, their tests, and the docs entry, on
`main`.

## Actual behavior

None of the above exist on `main`. Only the incidental tooling changes
landed.

## Suggested fix

1. Re-apply the dropped diff. The full intended change is recoverable
   directly from commit `5a93edd` (`git show 5a93edd -- contracts/
   docs/error-codes.md` gives the exact patch) — this shouldn't need to be
   re-implemented from scratch, just cherry-picked/re-applied on top of
   current `main` and reconciled with whatever else has landed in
   `attester-registry/src/lib.rs` since (including the fixes from #02,
   since `5a93edd`'s `lib.rs` diff was presumably based on a
   still-broken or differently-broken version of the file).
2. Before merging the reapplied change, confirm #01 is fixed so `cargo
   test` actually runs and the 12 tests this feature claims execute in
   CI.
3. Treat this as a signal to audit other recently-merged PRs for the same
   failure mode — if this happened once undetected, it's worth a quick
   `git diff <pr-branch> <merge-commit>^2..<merge-commit>` sanity check
   (or equivalent) across recent merges rather than assuming this is the
   only instance.

## Acceptance criteria

- [ ] `add_attesters`/`remove_attesters` (or a deliberately reconsidered
      replacement, if a maintainer decides the batch design should change
      given time has passed) exist on `main`, admin-gated, budget-bounded,
      idempotent, as originally specified.
- [ ] Tests from `5a93edd` (or their equivalent) pass in CI.
- [ ] `docs/error-codes.md` documents `BatchTooLarge`.
- [ ] A short note added to `CHANGELOG.md` under `[Unreleased]`, per
      `CONTRIBUTING.md`'s rule that contract-behavior changes need a
      changelog entry — this one arguably needs it twice: once for
      landing late, and to avoid future confusion about when the feature
      actually shipped.

## Environment

- Contract(s) affected: attester-registry
- Reference commit for the original (dropped) implementation: `5a93edd`
- Merge commit that dropped it: `245d0f9`
