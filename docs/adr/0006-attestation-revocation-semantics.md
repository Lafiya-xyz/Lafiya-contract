# ADR: Attestation Revocation Semantics

## Status
Accepted

## Context
Currently, `attest` in `contracts/attestation-registry/src/lib.rs` stores a
new `Attestation { attester, timestamp }` entry per call. `remove_attester` and
`suspend_attester` in `contracts/attester-registry/src/lib.rs` modify allowlist
state only in that contract's own storage; they have no cross-contract effect on
previously recorded attestations.

We need to define the trust model that governs how a responder should interpret
an attestation when the attesting CHW has since been removed or suspended:

- **Option A — Attestations are immutable historical facts.** A past attestation
  proves that a then-trusted party verified the record at a given time.
  Responders independently check the attester's *current* status via a second
  call to `is_attester`; the attestation record itself never changes.
- **Option B — Attestations are retroactively invalidated** when the attester is
  removed. `get_attestation` would cross-call `attester-registry.is_attester` at
  read time, returning the attestation only when the attester is still active.

## Decision

We adopt **Option A** for the pre-alpha milestone: attestations are immutable
historical records, and the trust anchor they represent is the fact that *at
the recorded timestamp* a then-allowlisted CHW attested to the record.

Rationale:

1. **Separation of concerns.** The attestation registry is a tamper-evident
   append-only log (per ADR-0001). Adding a live cross-contract dependency to
   every read path changes its character from "ledger of historical facts" to
   "real-time trust oracle" — a different, more complex contract with more
   failure modes (cross-contract call failures, gas spikes on reads).

2. **Queryable current status.** Responders who need to know whether an attester
   is *still* trusted call `attester-registry.is_attester(attestation.attester)`
   independently. Both pieces of information — "this was attested" and "the
   attester is currently trusted" — are available on-chain; combining them is a
   presentation-layer concern, handled in `lafiya-web`.

3. **Admin-gated explicit revocation is available.** `revoke_attestation` exists
   for cases where a specific attestation must be erased (e.g. an error in the
   record hash, a demonstrably fraudulent attestation that must not appear in
   any audit trail). This is a deliberate, per-record admin action, not an
   automatic side effect of allowlist management.

4. **Operational simplicity for pre-alpha.** The CHW population is small and
   admin-supervised. Fraudulent attestations can be handled with explicit
   `revoke_attestation` calls, and the responder-facing UI (lafiya-web) can be
   updated to display both attestation history and current attester status
   without requiring on-chain logic changes.

This is an **explicit decision, not an accidental default**. If the operational
model changes — for example, if attestations should automatically be hidden when
the attesting CHW is suspended — this ADR must be revisited and superseded with
a new design that specifies the read-path cross-contract call, its failure
semantics, and its gas impact (see Consequences below).

## Consequences

### Positive
- `get_attestation` remains a simple, cheap, single-contract read with no
  cross-contract call and no new failure modes.
- The contract's append-only audit trail property (ADR-0001) is preserved.
- Admin-gated explicit revocation (`revoke_attestation`) provides a surgical
  tool for correcting specific records without affecting the rest.

### Negative / Trade-offs
- A responder querying only `get_attestation` receives no signal that the
  attesting CHW has since been removed or suspended. The `lafiya-web` UI must
  explicitly check and display current attester status alongside the attestation
  to avoid misleading users.
- There is no bulk "invalidate all attestations by this CHW" operation at the
  contract level. An operator discovering a fraudulent CHW must enumerate that
  CHW's record hashes via the `AttestationRecorded` event log and issue
  individual `revoke_attestation` calls or build a batch CLI tool. This is
  operationally expensive for a high-volume fraudulent attester.
- Storing historical attestations from removed attesters consumes persistent
  storage rent indefinitely (until explicitly revoked). For a small CHW
  population this is negligible; it should be re-evaluated if the attester set
  grows significantly.

## Follow-up

- `lafiya-web` must be updated to display current attester status (active /
  suspended / removed) alongside each attestation in the verification display,
  so responders see both "attested by X at time T" and "X is [currently active |
  suspended | no longer registered]".
- A CLI helper for bulk revocation by attester (enumerating `AttestationRecorded`
  events and calling `revoke_attestation` for each matching hash) should be
  considered if the operational need arises.
- This decision should be reviewed before any mainnet deployment and referenced
  in the security threat model.
