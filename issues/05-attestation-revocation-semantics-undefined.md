---
title: "[feature]: attestation revocation semantics are undecided and unimplemented — a removed/suspended attester's past attestations still read as valid"
labels: enhancement, priority:p1, architecture, security
---

> Architect-tier trust-model question. Overlaps with the `is_attester`
> rewrite required by #02 — recommend resolving the state model and the
> compile fix together rather than patching `is_attester` twice.

## Problem

`docs/adr/0006-attestation-revocation-semantics.md` exists but was never
actually written — its "Decision" section is still template
scaffolding:

```
## Decision
[هنا غتكتب القرار ديالك: واش خاصهم يتمسحو (Invalid) ولا يبقاو (Valid)؟]
مثال: "We propose that upon removing an attester, their past
attestations should be considered invalid to maintain trust model
integrity."
```

(Status: "Proposed," not "Accepted.") In other words, the team correctly
identified this as an open question worth an ADR, but never actually
answered it — and the code has, by default, picked the *not revoked*
answer, silently:

- `attestation-registry::get_attestation(record_hash)` returns whatever
  `Attestation { attester, timestamp }` was last stored, with **no check
  against the attester's current status** in `attester-registry`. It
  never calls back into `AttesterRegistryClient` at read time — only
  `attest()` (the write path) checks `is_attester` at the moment of
  attestation.
- `attester-registry::remove_attester` and `suspend_attester` only touch
  `attester-registry`'s own storage. Neither one notifies
  `attestation-registry`, and `attestation-registry` has no mechanism to
  be notified even if they did (no admin function to bulk-invalidate, no
  event listener contract-side).

Concretely: attester `A` is allowlisted, attests to `record_hash` (an
emergency card gets marked verified). `A` is later discovered to be
fraudulent or compromised and is removed (or suspended, once that's
fixed per #02) from `attester-registry`. Any responder scanning that same
QR code and calling `get_attestation(record_hash)` still gets back a
successful `Attestation { attester: A, timestamp: ... }` with no
indication `A` is no longer trusted. This is exactly the CHW-fraud
scenario the ADR's own "Consequences" section names as the motivating
positive case for revocation ("Increased security, CHW fraud
prevention") — the protection the ADR describes doesn't exist yet.

`revoke_attestation(record_hash)` does exist, but it's a manual, per-hash
admin action — there's no bulk "invalidate everything this attester ever
signed" operation, so cleaning up after a compromised attester means the
admin must somehow enumerate every `record_hash` that attester touched
(which `attestation-registry`'s storage layout — keyed by `record_hash`,
not indexed by attester — doesn't support querying for on-chain at all).

## Proposed change

This needs a maintainer decision, not just an implementation — the ADR
should actually be filled in as part of this issue, not deferred again.
Two shapes worth considering explicitly:

1. **Attester-status-checked reads.** `get_attestation` (or a new
   `get_attestation_verified` that layers on top of the existing one,
   preserving the current function for callers who want the raw record)
   cross-calls `attester_registry.is_attester(attestation.attester)` at
   read time and reports whether the attester is *currently* allowlisted
   alongside the stored attestation. Cheapest to implement, doesn't
   require attester-registry to know about attestation-registry, but adds
   a cross-contract call to every read and doesn't distinguish "removed
   after attesting" from "was never valid" in the returned data unless the
   response shape is extended.
2. **Explicit bulk revocation.** Add an admin-gated
   `revoke_all_by_attester(attester: Address)` on `attestation-registry`
   — but this requires either an attester → `record_hash[]` index (a
   storage-shape change with real cost implications worth benchmarking
   against `docs/storage-cost.md`'s existing methodology) or accepting
   that it can only be enumerated off-chain via the event log
   (`AttestationRecorded` events, per `docs/architecture/
   event-indexing.md`) and replayed as a batch of individual
   `revoke_attestation` calls from the CLI/indexer.

Given `lafiya-cli` and the event-indexer design already lean on
off-chain event replay for other things, option 2's off-chain-enumeration
variant is likely the pragmatic fit — but that's a call for whoever owns
the trust-model tradeoff here, not something to decide by default via
omission the way it's effectively been decided so far.

## Alternatives considered

- **Leave attestations immutable forever regardless of attester status**
  (the current de facto behavior). Valid position if the intent is "an
  attestation is a historical fact, responders must independently verify
  attester status via a separate call" — but if that's the real decision,
  it should be the ADR's actual written decision, and
  `lafiya-web`/responder-facing docs need to say so explicitly, since the
  current `get_attestation` shape gives no hint that the caller needs a
  second lookup.

## Scope

- Component(s) affected: attestation-registry, attester-registry, and
  cross-repo — `lafiya-web`'s QR/verification display almost certainly
  needs to change however this is decided, since it's presumably the
  primary consumer of `get_attestation`.
- Does this change a contract's public function signatures or the
  attestation schema? **Yes, likely** — either `get_attestation`'s return
  shape changes (option 1) or a new function is added (option 2). Per
  `CONTRIBUTING.md`'s cross-repo rule, this must be flagged to
  `lafiya-web` maintainers before implementation, not just at PR time.

## Acceptance criteria

- [ ] `docs/adr/0006-attestation-revocation-semantics.md`'s "Decision"
      section is actually filled in and the status moved to "Accepted"
      (or superseded by a new ADR, if the team prefers).
- [ ] The chosen mechanism is implemented and tested, including a test
      that specifically reproduces the fraud scenario: attester attests,
      gets removed/suspended, and the read path reflects the new decision
      (either shows revoked, or is documented as intentionally not
      checking, with responder-facing docs updated to match).
- [ ] Cross-repo impact flagged to `lafiya-web` per `CONTRIBUTING.md`.

## Additional context

- [ADR-0006](../docs/adr/0006-attestation-revocation-semantics.md)
- Related: the `is_attester` rewrite needed for #02 (this issue's decision
  should inform, or be informed by, exactly what "suspended" is supposed
  to mean relative to "removed" — right now the code has two different
  admin actions, `suspend_attester` and `remove_attester`, and no ADR
  distinguishing their intended effect on past attestations either).
