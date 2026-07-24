---
title: "[audit] ARCH-02: attestation revocation semantics undecided — a removed/suspended attester's past attestations still verify"
labels: enhancement, priority:p1, architecture, security, severity:high
---

**Severity:** High
**Difficulty:** Low — requires only that an attester be removed/suspended after attesting, which is the intended normal operational response to discovering fraud
**Type:** Business Logic / Trust-Model Flaw

> Overlaps with BUILD-01b (`is_attester` rewrite) — resolve the state
> model once, in the same change, rather than patching `is_attester`
> twice.

## Summary

`docs/adr/0006-attestation-revocation-semantics.md` poses exactly this
question and was never answered — its Decision section is still template
scaffolding, status "Proposed." The code has, by default, implemented the
answer "attestations are never revoked by attester status change," and
does so silently: `get_attestation` performs no liveness check against
the attester's current allowlist/suspension status. This directly
defeats the fraud-prevention property the ADR itself names as the
motivating case for having revocation semantics at all.

## Location

- Decision gap: `docs/adr/0006-attestation-revocation-semantics.md:4,9-11`
- Read path with no status check:
  `contracts/attestation-registry/src/lib.rs:218-222` (`get_attestation`)
- Write path that checks status only at write time:
  `contracts/attestation-registry/src/lib.rs:149-186` (`attest`)
- Removal path with no downstream effect:
  `contracts/attester-registry/src/lib.rs:182-192` (`remove_attester`),
  `195-202` (`suspend_attester`)

## Technical Detail

```
4	- **Status:** Proposed
...
9	## Decision
10	[هنا غتكتب القرار ديالك: واش خاصهم يتمسحو (Invalid) ولا يبقاو (Valid)؟]
11	مثال: "We propose that upon removing an attester, their past
    attestations should be considered invalid to maintain trust model
    integrity."
```

`get_attestation(record_hash)` returns whatever `Attestation { attester,
timestamp }` was last stored, unconditionally — no cross-call into
`attester-registry` to check current status. Only `attest()` (the write
path, line ~154 `attester.require_auth()` through the `is_allowlisted`
check) verifies allowlist membership, and only at the moment of writing.
`attester-registry::remove_attester` and `suspend_attester` touch only
`attester-registry`'s own storage; there is no notification mechanism,
event listener, or cross-contract call informing `attestation-registry`
that an attester it previously trusted is no longer trusted.

## Proof of Concept (Attack / Failure Trace)

1. Attester `A` is allowlisted in `attester-registry`.
2. `A` calls `attest(A, record_hash)` — succeeds, `Attestation { attester:
   A, timestamp: T }` is stored.
3. `A` is later discovered to be fraudulent (falsely attesting to
   unverified emergency records) and is removed via
   `attester-registry::remove_attester(A)`.
4. A responder scans the QR code encoding `record_hash` and calls
   `attestation-registry::get_attestation(record_hash)`.
5. **Result:** the call succeeds and returns `Attestation { attester: A,
   timestamp: T }` with no indication `A` is no longer trusted. The
   responder has no on-chain signal that this attestation should now be
   treated as invalid.

This is precisely the scenario ADR-0006's own Consequences section names
as the positive case for revocation ("Increased security, CHW fraud
prevention") — the protection does not exist.

`revoke_attestation(record_hash)` exists but is a manual, per-hash admin
action. There is no bulk "invalidate everything `A` ever attested to"
operation, and `attestation-registry`'s storage is keyed by
`record_hash`, not indexed by attester — there is no on-chain way to
enumerate what needs revoking after removing a compromised attester.

## Impact

The core trust guarantee this system exists to provide — "a responder can
independently verify that a currently-trusted party attested to this
record" — silently degrades to "a responder can verify *someone who was
once trusted* attested to this record, with no way to know if that trust
has since been revoked." For a system explicitly designed around
CHW-fraud prevention (per the ADR), this is a trust-model-defeating gap,
not a cosmetic one.

## Recommendation

Requires a maintainer decision, not a default. Two concrete shapes:

1. **Status-checked reads.** `get_attestation` (or a new
   `get_attestation_verified` layered on top, preserving the raw-record
   function) cross-calls `attester_registry.is_attester(attestation.attester)`
   at read time. Cheapest to implement; adds a cross-contract call to
   every read; does not distinguish "removed after attesting" from "never
   valid" unless the response shape is extended.
2. **Explicit bulk revocation.** Admin-gated
   `revoke_all_by_attester(attester: Address)`, requiring either an
   attester → `record_hash[]` on-chain index (real storage-cost impact,
   should be benchmarked per `docs/storage-cost.md`'s methodology) or
   off-chain enumeration via the `AttestationRecorded` event log, replayed
   as batched `revoke_attestation` calls from the CLI/indexer.

Given `lafiya-cli` and the event-indexer design already depend on
off-chain event replay elsewhere, option 2's off-chain-enumeration variant
is the pragmatic default — but this is a trust-model tradeoff for a
maintainer to own explicitly, not to inherit by omission.

## Alternatives Considered

**Leave attestations immutable regardless of attester status** (current
de facto behavior). Defensible only as an explicit decision — "an
attestation is an immutable historical fact; responders must
independently verify current attester status via a second call" — but if
so, `get_attestation`'s current shape gives no signal that a second
lookup is required, and `lafiya-web`'s verification display needs to say
so explicitly to end users.

## Scope / Cross-Repo Impact

Likely changes `get_attestation`'s public shape or adds a new function —
per `CONTRIBUTING.md`'s cross-repo rule, this must be flagged to
`lafiya-web` maintainers before implementation, since it is presumably
the primary consumer of this read path.

## Verification

- [ ] ADR-0006's Decision section is filled in and Status moved to
      Accepted (or superseded by a new ADR).
- [ ] The chosen mechanism is implemented and tested, including a test
      reproducing the attack trace above and asserting the read path
      reflects the decision.
- [ ] Cross-repo impact flagged to `lafiya-web` per `CONTRIBUTING.md`.
