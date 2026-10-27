# ADR 0013: Multi-Attester Quorum Attestations

- **Status:** Proposed
- **Date:** 2025-01-01
- **Deciders:** Attestation Registry maintainers, Verification Model WG
- **Related:** Issue #545, `docs/specs/verification-model.md`, `contracts/attestation-registry/src/lib.rs`

## Context

Today a single attester can mark a record (e.g. a card's genotype field) as verified. A single compromised or negligent attester is therefore sufficient to produce a "verified" result for high-assurance fields. Critical record classes (genotype, blood type, rare-disease markers) warrant independent corroboration from multiple distinct attesters before the registry reports the record as verified.

We need a quorum policy that:

- is configurable per record class,
- aggregates co-attestations by **distinct** attesters,
- exposes only public aggregate data (count + policy satisfaction) in the verification view,
- has well-defined interplay with revocation and supersession.

## Decision

### 1. Quorum policy per record class

Introduce a `QuorumPolicy` keyed by record class:

```rust
pub struct QuorumPolicy {
    pub record_class: RecordClass,
    /// Minimum number of distinct attesters required for "Verified".
    pub min_attesters: u32,
    /// Optional cap on attesters counted toward quorum (prevents unbounded growth).
    pub max_counted: Option<u32>,
    /// Whether the original attester counts toward the quorum (default: true).
    pub include_originator: bool,
}
```

Policies are stored in contract storage and are admin-configurable. Default policy for all classes is `min_attesters = 1` (preserves current behaviour). High-assurance classes (e.g. `RecordClass::Genotype`) default to `min_attesters = 2`.

### 2. Distinct-attester rule

Co-attestations are aggregated by the **attester identity** (the signing key / attester address), not by attestation record. The rule:

- A given attester contributes **at most one** count toward quorum for a given record hash, regardless of how many attestation entries they create.
- A second attestation from the **same** attester for the same record hash is treated as a **no-op** with respect to quorum counting. It does not error (to keep idempotent retries safe) but it does not increment the distinct-attester count. The contract emits a `DuplicateAttesterIgnored` event for observability.
- Distinctness is evaluated against the *set* of currently-active (non-revoked, non-superseded) attestations for the record hash.

### 3. Verification view

The verification view returns an aggregate that does not leak more than public data:

```rust
pub struct VerificationView {
    pub state: VerificationState,        // Verified | Pending | Revoked | Superseded | Disputed
    pub distinct_attesters: u32,         // count of distinct active attesters
    pub required_attesters: u32,         // policy.min_attesters
    pub policy_satisfied: bool,          // distinct_attesters >= required_attesters
}
```

The view **must not** enumerate attester identities, timestamps, or per-attester metadata. Only the aggregate count and policy satisfaction are exposed. This bounds information leakage to what is needed for a verifier to decide whether to trust the record.

### 4. Interplay with revocation

- Revoking an attestation removes that attester from the distinct-attester set for the record hash.
- If, after revocation, `distinct_attesters < required_attesters`, the record's state transitions from `Verified` to `Pending` (quorum no longer satisfied).
- **Monotonicity property (to be property-tested):** for any sequence of revocations, `distinct_attesters` is non-increasing, and once `policy_satisfied` becomes `false` it cannot become `true` again without a *new* distinct attestation being added. Formally: revocation never increases the distinct-attester count.
- Revocation of one attester's attestation does not affect other attesters' contributions.

### 5. Interplay with supersession

- Supersession replaces a record hash with a new one. Quorum is tracked **per record hash**. A superseded record hash's quorum state is frozen (state = `Superseded`) and no longer counts toward any live verification.
- Co-attestations attached to the old hash do **not** carry over to the new hash. Attesters must re-attest the new hash to satisfy its quorum. This prevents a supersession from silently inheriting quorum from a different (possibly weaker) attestation set.
- The verification view for a superseded hash returns `state = Superseded` and the frozen counts at time of supersession.

## Acceptance criteria mapping

- **Distinct-attester rule enforced; same attester twice is a no-op or error as specified.** → §2: same attester twice is a no-op (idempotent), emits `DuplicateAttesterIgnored`.
- **Property tests for quorum monotonicity under revocation.** → §4: `distinct_attesters` non-increasing under revocation; `policy_satisfied` cannot flip false→true without a new distinct attestation. Property tests to be added in the implementation PR.
- **ADR merged before implementation.** → This document.

## Consequences

- Adds storage for per-class policies and per-record distinct-attester sets.
- Verification view shape changes; SDK and `verification-model.md` must be updated in lockstep.
- Existing single-attester records remain valid under the default `min_attesters = 1` policy.
- Gas cost of co-attestation grows with the number of distinct attesters, bounded by `max_counted`.

## Alternatives considered

- **Count attestations rather than distinct attesters.** Rejected: a single attester could satisfy quorum by submitting multiple entries.
- **Error on duplicate attester.** Rejected: breaks idempotent retries and complicates client UX; no-op with event is safer.
- **Carry quorum across supersession.** Rejected: allows quorum laundering across record versions.