# ADR: Attestation Revocation Semantics

## Status
Proposed

## Context
Currently, `attest` in `contracts/attestation-registry/src/lib.rs` overwrites prior attestations. `remove_attester` does not retroactively affect past attestations. We need to define if past attestations should remain valid or be revoked when an attester is removed.

## Decision
[هنا غتكتب القرار ديالك: واش خاصهم يتمسحو (Invalid) ولا يبقاو (Valid)؟]
مثال: "We propose that upon removing an attester, their past attestations should be considered invalid to maintain trust model integrity."

## Consequences
- **Positive:** Increased security, CHW fraud prevention.
- **Negative:** Complexity in query processing for `get_attestation`.
