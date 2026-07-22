---
title: "[bug]: design docs and the attester-load test have drifted from actual contract behavior — false confidence in coverage that doesn't exist"
labels: bug, priority:p2, documentation, test-coverage
---

> Architect-tier: each item below is individually minor, but together
> they're the same failure pattern as #01/#03 at smaller scale —
> something that looks like a safety net (a benchmark, a load test, a
> design spec, an error-code reference) has quietly stopped reflecting
> reality, and nothing has been checking. Grouped as one issue since the
> fix for all four is "make the artifact honest and re-sync it," not four
> separate code changes.

## Description

Four artifacts in this repo exist specifically to give confidence about
behavior or cost, and all four have drifted from what the contracts
actually do:

### 1. `contracts/attester-registry/src/large_test.rs` asserts nothing about budget

```rust
// Record resource budget usage (debug output for CI logs)
let budget = env.budget();
// These methods exist on Budget; if not, they are placeholders for illustration.
// In actual Soroban SDK, you can query used CPU instructions and memory.
// For now we just ensure the test completes without hitting limits.
println!("Budget after adding {} attesters: {:?}", total_attesters, budget);
```

The test's own comments admit it: this doesn't assert a CPU/memory
ceiling, it just prints the budget and relies on the test *not panicking*
from resource exhaustion as its only signal. A regression that
significantly increases per-attester cost (e.g. an accidental O(n) scan
introduced somewhere) would have to roughly reach Soroban's hard resource
limit before this test would ever fail — anything short of that passes
silently. For a test explicitly named "load test" whose purpose is
catching cost regressions at scale, that's a false sense of coverage.

### 2. `docs/storage-cost.md` benchmarks aren't wired to anything

The doc says "You can re-run these benchmarks using... `make bench`,"
and `make bench` is `cargo test -- --nocapture` — i.e. it just runs the
test suite with stdout visible, including `large_test.rs`'s `println!`
above. There's no script that regenerates the table in
`docs/storage-cost.md` from that output, and no check that the numbers in
the doc still match reality. The 1000-attester row's cost figures could
be arbitrarily stale (they predate at least the TTL and suspend/reinstate
work per the commit history) and nothing would notice.

### 3. `docs/architecture/event-indexing.md`'s event list is stale

```
Lafiya contracts emit the following events on-chain:
- AttesterAdded
- AttesterRemoved
- AttestationRecorded
```

Actual events currently declared across both contracts (per
`#[contractevent]` in each `lib.rs`): `AdminTransferred` (both
contracts), `Initialized`, `AttesterAdded`, `AttesterRemoved`,
`AttesterSuspended`, `AttesterReinstated`, `AttestationRecorded`,
`AttestationRevoked` — the design doc is missing five of eight. This
matters beyond "doc is out of date": this spec is what's supposed to
drive the event-indexer that keeps `lafiya-web`'s displayed verification
status in sync with on-chain state (per the doc's own stated purpose).
An indexer built strictly from this spec would never process
`AttesterSuspended`, `AttesterReinstated`, or `AttestationRevoked` —
silently compounding the revocation-semantics gap in issue #05, since
even a *correct* fix to that issue's on-chain logic wouldn't reach
`lafiya-web` if the indexer spec never told it those events exist.

### 4. `docs/error-codes.md` doesn't cover `multisig-account` at all

The doc's two sections are `attester-registry` and `attestation-registry`
only. `contracts/multisig-account/src/lib.rs` defines its own
`#[contracterror] pub enum Error` with six variants
(`InvalidThreshold`, `DuplicateSigner`, `NotEnoughSigners`,
`BadSignatureOrder`, `UnknownSigner`, `NotInitialized`) — none
documented. `attestation-registry/src/test.rs` has a
`test_error_codes_are_documented` test that enforces this doc stays in
sync *for the two contracts it checks*, but nothing equivalent covers
`multisig-account`, so this gap has no test to catch it even after #01
(CI) is fixed.

## Proposed fix

- `large_test.rs`: add an actual assertion — e.g. capture
  `budget.cpu_instruction_cost()` (or the equivalent current-SDK accessor)
  after 10, 100, and 1000 attesters and assert each stays under an
  explicit, documented ceiling with headroom, so a real regression fails
  the test rather than just printing a bigger number.
- Either automate `docs/storage-cost.md` generation from the load test's
  output (a small script parsing test output into the markdown table,
  run in CI or pre-commit) or, more cheaply, add a one-line note to the
  doc stating the table is manually maintained and the date/commit it was
  last verified against, so staleness is at least visible rather than
  silent.
- Update `docs/architecture/event-indexing.md`'s event list to the actual
  current set, and add a short line asking future contract PRs that add
  `#[contractevent]` types to update this doc (or, better, extend the
  existing `test_error_codes_are_documented` pattern with a sibling test
  that greps for `#[contractevent]` structs and asserts each is named in
  this doc — same mechanism, same payoff, already has precedent in this
  codebase).
- Add a `## \`multisig-account\`` section to `docs/error-codes.md` with
  its six error variants, and extend
  `test_error_codes_are_documented`-style coverage to include
  `multisig-account` (currently that test only reads
  `attester-registry`/`attestation-registry` source paths).

## Acceptance criteria

- [ ] `large_test.rs` asserts a concrete budget ceiling per allowlist
      size tested, not just "didn't panic."
- [ ] `docs/storage-cost.md` either regenerates automatically or is
      explicitly marked with a last-verified reference point.
- [ ] `docs/architecture/event-indexing.md` lists all currently-declared
      events.
- [ ] `docs/error-codes.md` documents `multisig-account`'s `Error` enum,
      and doc-sync test coverage extends to it.

## Environment

- Affected: `contracts/attester-registry/src/large_test.rs`,
  `docs/storage-cost.md`, `docs/architecture/event-indexing.md`,
  `docs/error-codes.md`, `contracts/multisig-account/src/lib.rs`
