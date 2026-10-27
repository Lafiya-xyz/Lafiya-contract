# Attester capability tiers

- **Status:** Normative (issue #588)
- **Implementation:** `contracts/attester-registry/src/tiers.rs`,
  `contracts/attestation-registry` (`attest_with_scope`)

Not every verification needs the same qualification. A tier says what an attester is, and a
capability matrix says which attestation classes each tier may attest.

## Tiers

`Tier` is ordered: `Chw (1) < Nurse (2) < Physician (3)`. A higher tier satisfies any lower
requirement. `set_attester_tier` (global registrar) assigns one; an attester without an
assignment is `Chw`, so every existing attester keeps working.

## Capability matrix (data, not constants)

The matrix is a `Map<Symbol, Tier>` of attestation class to the **minimum** tier. Classes
that are not listed require only `Chw`, so an empty matrix, the default, changes nothing.
Examples of classes are `identity`, `registration`, `vitals` and `medication`; the set is
whatever the programme puts in the matrix (at most `MAX_TIER_CLASSES = 32`).

Changes are **timelocked and evented**:

1. `propose_tier_matrix(matrix)` (admin) stores a pending proposal and emits
   `TierMatrixProposed { executable_at, entries }`. A new proposal replaces the old one.
2. After `TIER_MATRIX_TIMELOCK_SECONDS` (48 hours), `apply_tier_matrix()` (admin) swaps it in
   and emits `TierMatrixApplied`.
3. `cancel_tier_matrix()` (admin) discards the proposal and emits `TierMatrixCancelled`.

Tier assignments emit `AttesterTierSet`. Tier is cleared when an attester is removed and
moves with the attester on rotation.

## Enforcement

`attestation-registry.attest_with_scope(attester, patient, record_hash, class, facility_id)`
calls `attester-registry.is_class_allowed(attester, class)` (allowlisted **and** tier at or
above the matrix minimum) and fails with `AttesterTierInsufficient` otherwise. The class
is stored with the attestation and readable via `get_attestation_scope`. The older
`attest`, `attest_versioned`, `attest_version` and `batch_attest` carry no class and are not
tier-gated, which is what keeps existing attesters working.

Enforcement happens at attestation time only. Tightening the matrix later does not
invalidate existing attestations.

## Display in the verdict

The verification model ([`verification-model.md`](verification-model.md), section 2.1)
treats `attester_tier` and `attestation_class` as annotations: verifiers display the
attester's tier (`CHW`, `Nurse`, `Physician`) and the attestation class beside the verdict
and **do not** let them change it. Suggested rendering: "Verified by a Nurse (vitals)".
