# ADR-0012: Per-attestation blinded commitments

- **Status:** Proposed
- **Date:** 2026-09-24
- **Deciders:** Maintainers and relevant reviewers

> **Terminology:** shared terms (*attester*, *record commitment*, *schema version*, …) are
> defined canonically in the [glossary](../glossary.md). Link there rather than redefining them.

## Context

A card's *record commitment* appears in its QR code and repeats on-chain on every
re-attestation and revocation. Anyone who scans a card once can watch the ledger for that
hash and learn when and where (via the attester's region) the patient is verified. The
analysis in [`docs/security/linkability.md`](../security/linkability.md) rates this the
highest-severity linkability channel, and `scripts/poc/linkability_poc.py` demonstrates it.

## Decision

Each attestation commits to a fresh blinded value
`H(domain ‖ canonical_record ‖ nonce)`, where `nonce` is 16+ random bytes generated per
attestation and carried on the card issued for that attestation. The on-chain key is the
blinded value, so no two attestations share a hash. Verifiers recompute the blinded value
from the disclosed fields and the card's nonce. Revocation targets the blinded value.
This extends LRC-1 ([ADR-0008](0008-record-commitment-canonicalization.md)) as a new
commitment version rather than changing it in place.

## Alternatives considered

### Card-scoped rotating identifiers

Rotating the QR identifier without rotating the on-chain key only moves the linkable value,
and needs a resolver that becomes a tracking point itself.

### Batch anchoring

Hides timing inside a batch window, but needs Merkle proofs on cards, a separate revocation
structure, and larger contract changes. Kept as a research follow-up.

### Coarse timestamps and relayed submission

Each addresses a lower-severity channel only; neither stops "watch a hash". Relayed
submission remains a follow-up.

## Consequences

### Positive

- One card scan no longer reveals a patient's other attestations.
- No new infrastructure; the contract stores the same 32-byte key.

### Trade-offs and risks

- A new card per attestation; losing the card loses the nonce for that attestation.
- The registry can no longer return a record's history by one key; history is patient-held.
- The attester address and timestamp of the single attestation on a scanned card are still visible.

## Follow-up

See "Follow-up engineering issues" in [`docs/security/linkability.md`](../security/linkability.md).
