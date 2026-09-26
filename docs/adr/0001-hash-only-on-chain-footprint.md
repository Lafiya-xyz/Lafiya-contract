# ADR-0001: Keep health data off-chain and store only opaque commitments

- **Status:** Accepted
- **Date:** 2026-07-18
- **Deciders:** Lafiya contract maintainers

## Context

Lafiya needs a responder to verify that an authorized health worker attested to a specific
emergency record. The record itself can contain highly sensitive health information. Soroban
state is replicated and long-lived, so placing health facts, patient identifiers, public record
URLs, or encrypted record payloads on-chain would create unnecessary disclosure and retention
risk.

The contract layer only needs a stable value that binds an attestation to the off-chain record.
It does not need to read or interpret the record contents.

## Decision

No health-record contents or personal health facts will be stored by `Lafiya-contract`. A
patient's address is used transiently in a one-time consent grant bound to an attester and record
commitment; the grant is consumed when the attestation is written and the patient address is not
included in the attestation. Consent transaction inputs and the pending grant are publicly
observable on the ledger, so the grant is not a confidentiality mechanism.

`attestation-registry` accepts an opaque `BytesN<32>` `record_hash` and stores the latest
attestation under that key. On-chain state may contain:

- the opaque record commitment;
- the attester address and attestation timestamp;
- the record commitment, patient, and selected attester while a one-time consent grant is pending;
- the attester allowlist; and
- contract configuration such as administrator and registry addresses.

The off-chain Lafiya application is responsible for constructing the commitment. A plain hash
of predictable, low-entropy health fields is not automatically private: it can be vulnerable to
dictionary guessing. Commitment construction must therefore be defined by the off-chain data
model and threat model, including canonical serialization, domain separation, and secret
entropy or another hiding construction where required.

The contracts treat the commitment as an identifier only. A one-time consent grant proves that
the patient authorized the selected attester to attest that exact commitment; it does not prove
that the underlying health information is accurate, available, or current.

## Alternatives considered

### Store the health record directly on-chain

Rejected because it would expose sensitive data to every ledger participant, make practical
deletion impossible, increase storage cost, and exceed what the trust layer needs.

### Store an encrypted health record on-chain

Rejected because ciphertext creates permanent retention and key-management risk. Encryption
may later fail, keys may be leaked, and metadata remains public. The encrypted record belongs in
an access-controlled off-chain system.

### Keep attestations entirely in a centralized database

Rejected because responders and integrators would lose an independently checkable,
tamper-evident trust anchor.

## Consequences

### Positive

- The public contract surface minimizes sensitive data and regulatory exposure.
- A responder can independently check that an allowlisted address attested to a commitment.
- The contract remains agnostic to the shape and evolution of the off-chain health record.
- On-chain storage and execution remain small.

### Trade-offs and risks

- Lafiya still depends on the off-chain system for confidentiality, availability, and record
  presentation. The on-chain consent grant authorizes verification of a commitment, not the
  disclosure or accuracy of its underlying health data.
- A commitment proves integrity relative to a preimage; it does not prove medical correctness.
- Updating a record creates a different commitment, and previously published commitments cannot
  be erased from ledger history.
- Weak commitment construction can leak information through guessing attacks even though the
  raw fields are not stored on-chain.

## Follow-up

- Specify the canonical record-commitment construction in the web/data-model documentation.
- Cover commitment privacy and correlation risks in the project threat model.

## References

- [`README.md`](../../README.md), “Smart Contract Layer” and “Privacy & Compliance”
- [`contracts/attestation-registry/src/lib.rs`](../../contracts/attestation-registry/src/lib.rs)
