# ADR-0019: Facility registry linking attesters and attestations to licensed facilities

- **Status:** Proposed
- **Date:** 2026-10-17
- **Deciders:** Lafiya contract maintainers

> **Terminology:** shared terms (*attester*, *CHW*, …) are defined in the [glossary](../glossary.md).

## Context

An attestation names only the individual attester. A clinic that employs rotating staff
has no on-chain accountability, and a verifier cannot tell whether an attestation was made
on behalf of a licensed institution. Issue #592 asks for facilities as first-class
entities: admin-governed enrollment, attester affiliations, an optional facility id on
attestations validated against affiliation, a defined effect of affiliation changes on
existing attestations, and an enumeration/storage approach consistent with
[ADR-0017](0017-attester-enumeration-strategy.md).

## Decision

### A new contract: `facility-registry`

A separate Soroban contract (admin-governed, per [ADR-0003](0003-single-admin-initial-model.md)),
not a module of `attester-registry`. Facilities have a different lifecycle and governance
from the attester allowlist, and a separate contract keeps `attester-registry`'s ABI and
wasm size stable. `attestation-registry` reaches it through a local `#[contractclient]`
interface, the same pattern as [ADR-0002](0002-contractclient-boundary.md).

State: `Facility(id) -> {license_hash, region, enrolled_at, active, deactivated_at}`;
`Affiliation(id, attester) -> Vec<{since, until}>` (at most `MAX_PERIODS = 8`);
`AffiliateCount(id)` (at most `MAX_AFFILIATES_PER_FACILITY = 500` current); instance
`FacilityCount` (ids `1..=MAX_FACILITIES = 10,000`) and `MutationSeq`.

### Optional facility id on attestations

`attestation-registry.attest_with_scope(attester, patient, record_hash, class, facility_id)`
adds an optional `facility_id`. When present it must satisfy
`facility-registry.is_affiliated(facility_id, attester)` (the facility is active and the
attester's latest period is open) **at attestation time**; otherwise
`FacilityNotAffiliated`. A missing `FacilityRegistry` configuration fails with
`FacilityRegistryNotSet`. The scope is stored per attestation slot
(`AttestationScope(record_hash, sequence)`), evicted with the slot when history rolls over,
and cleared by a later attestation of the same slot made without scope. The existing
`attest*` entry points are unchanged, so existing integrations are unaffected.

### How affiliation changes affect existing attestations

An attestation is a historical fact about what the attester and facility were at that
time, so **changes never rewrite stored attestations.**

| Change | Effect on stored attestation | Effect on new attestations | Verifier action |
| --- | --- | --- | --- |
| `disaffiliate_attester` | None. Scope stays. | Refused for that facility. | Use `was_affiliated_at(facility, attester, attestation_ts)` to prove affiliation then. |
| `deactivate_facility` | None. | `is_affiliated` is false for all its attesters, so facility attestations are refused. | Show a warning from `is_facility_active`; the deactivation time is in `get_facility`. |
| `reactivate_facility` | None. | Open affiliations resume. | None. |
| Attester suspended or removed in `attester-registry` | None here. | Plain attester checks already refuse it. | Existing `AttesterSuspended`/`AttesterRemoved` verdicts apply. |

Period history is bounded: after `MAX_PERIODS` affiliations of the same pair, re-affiliating
returns `PeriodLimitReached`, which is a deliberate speed bump against affiliation churn.
Deactivation timestamps survive until reactivation; the full history is the event stream.

## Enumeration and storage cost

Consistent with ADR-0017 there is **no on-chain directory**. The facility list and each
facility's roster are derived off-chain from the events `FacilityEnrolled`,
`FacilityDeactivated`, `FacilityReactivated`, `AttesterAffiliated` and `AttesterDisaffiliated`.
Every mutation increments `MutationSeq` (`get_mutation_seq`) and carries the new value in
its event, so an indexer proves completeness with the recovery procedure in ADR-0017
unchanged. Per-facility roster size is available on-chain only as a count.

Storage cost, using Soroban entry sizes of roughly 100 bytes per `Facility`, 60 bytes per
period list entry and one `u32` counter:

| Item | Entries | Bound |
| --- | --- | --- |
| Facility records and counters | 2 per facility | 20,000 at `MAX_FACILITIES` |
| Affiliation lists | 1 per (facility, attester) pair | roughly attesters x facilities each works at; each at most 8 periods |
| Instance | 2 values (`FacilityCount`, `MutationSeq`) | constant |
| Attestation scope | 1 per scoped attestation slot | evicted with history (max 10 slots per record) |

Per-entry TTLs are extended on write. Reads (`is_affiliated`, `was_affiliated_at`) touch one
facility entry plus one affiliation entry, independent of roster size.

## Alternatives considered

- **Facility fields on `AttesterInfo`.** Cheap, but a single facility per attester, no
  history, and no governance separate from the allowlist.
- **Facility in the `Attestation` struct.** Changes the stored struct, breaking existing
  decoding and bindings; a side key avoids a migration.
- **On-chain roster enumeration.** Rejected by ADR-0017 for the same removal and rent
  reasons.

## Consequences

- One more contract to deploy, version and monitor; its release-manifest and bindings
  entries are follow-ups.
- `attestation-registry` gains one instance key, one persistent key family and four
  entry points. Storage layout is additive.
- Facility governance is single-admin for now; delegated facility managers are a
  follow-up.

## References

- Issue #592
- [ADR-0017](0017-attester-enumeration-strategy.md)
- `contracts/facility-registry`, `contracts/attestation-registry/src/scope_test.rs`
