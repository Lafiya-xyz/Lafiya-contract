---
title: "[audit] QA-01: safety-net artifacts (load test, cost benchmarks, event-indexing spec, error-code docs) have drifted from actual contract behavior"
labels: bug, priority:p2, documentation, test-coverage, severity:low
---

**Severity:** Low (individually); the pattern is the same class of failure as CI-01/PROC-01 at smaller scale
**Difficulty:** N/A — passive documentation/test drift, not an exploit
**Type:** Quality Assurance / Observability Gap

> Four independent findings grouped as one issue because the fix for all
> four is identical in kind — "make the artifact honest and re-sync it" —
> not four separate code changes. Each is evidence of the same root
> pattern already established by CI-01 and PROC-01: something that looks
> like a safety net has quietly stopped reflecting reality, unchecked.

## Finding QA-01a — Load test asserts nothing about resource cost

**Location:** `contracts/attester-registry/src/large_test.rs:50-55`

```rust
50	        // Record resource budget usage (debug output for CI logs)
51	        let budget = env.budget();
52	        // These methods exist on Budget; if not, they are placeholders for illustration.
53	        // In actual Soroban SDK, you can query used CPU instructions and memory.
54	        // For now we just ensure the test completes without hitting limits.
55	        println!("Budget after adding {} attesters: {:?}", total_attesters, budget);
```

The test's own comments concede it: no CPU/memory ceiling is asserted.
Its only pass/fail signal is whether the test panics from hitting
Soroban's hard resource limit outright. A regression that meaningfully
increases per-attester cost (e.g. an accidental O(n) scan) would have to
approach that hard limit before this test — explicitly named a "load
test" — would ever fail. Anything short of that passes silently. This is
false coverage: the test's existence implies a cost regression would be
caught; it would not be, short of a near-total budget blowout.

**Recommendation:** capture the relevant `Budget` accessor (current
`soroban-sdk` API — verify exact method name against the pinned version)
after 10, 100, and 1000 attesters and assert each stays under an
explicit, documented ceiling with headroom, so a real regression fails
the test.

## Finding QA-01b — Cost benchmarks in docs aren't wired to the test suite

**Location:** `docs/storage-cost.md` (entire document)

The doc states benchmarks are re-run via `make bench`, which is `cargo
test -- --nocapture` — it prints QA-01a's `println!` output to stdout.
No script regenerates the markdown table from that output, and nothing
checks the documented figures still match reality. The table predates at
least the TTL-extension and suspend/reinstate work per commit history and
could be arbitrarily stale.

**Recommendation:** either automate table generation from load-test
output (a small parsing script, run in CI or pre-commit) or, at minimum,
add a "last verified against commit `<sha>`" line so staleness is visible
rather than silent.

## Finding QA-01c — Event-indexing design spec omits 5 of 8 currently-declared events

**Location:** `docs/architecture/event-indexing.md:5-8`

```
5	Lafiya contracts emit the following events on-chain:
6	- `AttesterAdded`
7	- `AttesterRemoved`
8	- `AttestationRecorded`
```

Actual `#[contractevent]` types currently declared across both
contracts: `AdminTransferred` (both contracts), `Initialized`,
`AttesterAdded`, `AttesterRemoved`, `AttesterSuspended`,
`AttesterReinstated`, `AttestationRecorded`, `AttestationRevoked` — the
spec lists 3 of 8. This is not merely stale documentation: this spec is
the stated design input for the event-indexer that keeps `lafiya-web`'s
displayed verification status synchronized with on-chain state. An
indexer implemented strictly from this spec would never process
`AttesterSuspended`, `AttesterReinstated`, or `AttestationRevoked` —
compounding ARCH-02 (revocation-semantics gap): even a correct on-chain
fix for that finding would not reach `lafiya-web` if the indexer's own
design spec never told it those events exist.

**Recommendation:** update the event list to the current set. Consider
extending the existing `test_error_codes_are_documented`-style
enforcement (see QA-01d) with a sibling check that greps for
`#[contractevent]` declarations and asserts each is named somewhere in
this doc — the codebase already has precedent for this pattern.

## Finding QA-01d — Error-code reference omits multisig-account entirely

**Location:** `docs/error-codes.md` (entire document, sections at lines
`8` and `16`)

The document has exactly two sections, `attester-registry` and
`attestation-registry`. `contracts/multisig-account/src/lib.rs:24-34`
declares its own `#[contracterror] pub enum Error` with six variants
(`InvalidThreshold`, `DuplicateSigner`, `NotEnoughSigners`,
`BadSignatureOrder`, `UnknownSigner`, `NotInitialized`) — none
documented. `attestation-registry/src/test.rs`'s
`test_error_codes_are_documented` enforces sync *only* for the two
contracts it reads source from; `multisig-account` has no equivalent
check, so this gap will persist even after CI-01 is fixed.

**Recommendation:** add a `## \`multisig-account\`` section documenting
its six variants (including the new `TooManySigners` variant proposed in
SEC-01, once added), and extend `test_error_codes_are_documented`-style
coverage to include `multisig-account`'s source path.

## Verification

- [ ] `large_test.rs` asserts a concrete budget ceiling per allowlist
      size tested, not merely "did not panic."
- [ ] `docs/storage-cost.md` either regenerates automatically or carries
      an explicit last-verified reference point.
- [ ] `docs/architecture/event-indexing.md` lists all currently-declared
      events.
- [ ] `docs/error-codes.md` documents `multisig-account`'s `Error` enum,
      and doc-sync test coverage extends to it.
