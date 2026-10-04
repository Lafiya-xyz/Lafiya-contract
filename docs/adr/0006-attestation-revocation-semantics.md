# ADR: Attestation Revocation Semantics

## Status
Accepted — amended for compromise cutoffs

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

Attestations remain immutable historical records. In addition, an attester
registry suspension records a ledger timestamp cutoff. The new
`is_attestation_trusted` read checks the latest attestation against that
cutoff: attestations made before the cutoff remain historically valid, while
attestations made at or after it are untrusted. Raw reads and histories remain
available for auditability. Reinstating an attester does not erase the
cutoff; an administrator must explicitly update the attester's trust state
through a reviewed lifecycle operation.

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

3. **Explicit withdrawal and revocation are available.** An attester can
   withdraw their own verification without affecting other attestations for
   the hash. Admin `revoke_attestation` marks the entire hash revoked while
   preserving the historical attestations. `get_attestation_status` lets a
   responder distinguish `Revoked` from `NeverAttested`.

4. **Operational simplicity for pre-alpha.** The CHW population is small and
   admin-supervised. Fraudulent attestations can be handled with explicit
   `revoke_attestation` calls, and the responder-facing UI (lafiya-web) can be
   updated to display attestation state and current attester status
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
- Per-attester withdrawal leaves other verifications intact, while
  `get_attestation_status` preserves an explicit revoked signal after
  administrative revocation.

### Negative / Trade-offs
- A responder querying only `get_attestation` receives no signal that the
  attesting CHW has since been removed or suspended. The `lafiya-web` UI must
  explicitly check and display current attester status alongside the attestation
  to avoid misleading users.
- The cutoff is checked when a consumer asks for trust, rather than deleting
  records. Consumers must use `is_attestation_trusted` (or apply the same
  cutoff rule) instead of treating a raw historical read as a current trust
  decision.
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
