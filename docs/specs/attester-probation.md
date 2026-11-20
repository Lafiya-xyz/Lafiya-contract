# Attester probation

- **Status:** Normative (issue #589; objective promotion criteria added in issue #678)
- **Implementation:** `contracts/attester-registry/src/probation.rs`

Newly enrolled attesters are constrained for a trial period, or until they meet the
objective promotion criteria (or a reviewer promotes them early). Probation is **off by
default**: until `set_probation_config` is called with a non-zero `duration`, no attester
is on probation.

## Policy

`set_probation_config(duration, rate_cap, rate_window_ledgers, payout_eligible, criteria)`
(admin):

| Field | Meaning |
| --- | --- |
| `duration` | Seconds each new enrollment stays on probation; `0` disables probation for new enrollments. |
| `rate_cap` | Maximum attestations per `rate_window_ledgers` while on probation; `0` leaves the rate uncapped. A cap needs a non-zero window. |
| `payout_eligible` | Whether a probationary attester may receive `incentive-pool` payouts. |
| `criteria` | Versioned `ProbationPolicy` struct of objective promotion criteria (below). |

Caps are read live from the policy; each attester's end time is fixed at enrollment
(`ends_at = enrolled_at + duration`).

### Versioned promotion criteria (`ProbationPolicy`)

The promotion criteria are a versioned struct so that the meaning of a promotion is
unambiguous on-chain and to indexers. The stored policy carries a `version` field; a
criteria change bumps the version and emits `ProbationPolicyUpdated { version }`. An
attester's counters are always evaluated against the version that is current at the time
`try_promote` is called.

| Field | Meaning |
| --- | --- |
| `version` | Monotonic criteria version. |
| `min_attestations` | Minimum attestations recorded while on probation. |
| `min_tenure_secs` | Minimum time since enrollment. |
| `max_upheld_disputes` | Maximum upheld disputes tolerated (typically `0`). |
| `max_revocations` | Maximum revocations tolerated (typically `0`). |

All thresholds are inclusive lower bounds except the `max_*` fields, which are inclusive
upper bounds. A `min_*` of `0` and a `max_*` of `u32::MAX` disable that criterion.

### Per-attester counters

The registry keeps narrow counters per attester, updated only through one-way hooks so
the data flow stays acyclic:

| Counter | Updated by |
| --- | --- |
| `attestations` | `attestation-registry` calls `record_probation_attestation(attester)` after a successful attestation. |
| `upheld_disputes` | `attestation-registry` calls `record_probation_dispute(attester, upheld)` when a dispute resolves. |
| `revocations` | The registry increments its own counter in `revoke_attester`. |

Counters reset when a fresh probation starts (new enrollment) and are cleared with the
record on `remove_attester` / `remove_attesters`.

## States

`get_probation_state(attester)` is computed from the ledger timestamp:

| State | When |
| --- | --- |
| `Standard` | No record, promoted, or expired and not flagged. |
| `Probation` | Before `ends_at`, not flagged. |
| `Flagged` | Flagged, whenever it is (even after `ends_at`). Limits keep applying. |

**Automatic promotion at expiry** is lazy: at `ends_at` an unflagged attester reads as
`Standard` with no transaction. Anyone may call `finalize_probation` afterwards to clear
the record and emit `ProbationPromoted { early: false }`. A flagged attester is never
auto-promoted.

**Objective promotion** is `try_promote(attester)`, permissionless: anyone may call it.
It evaluates the current `ProbationPolicy` against the attester's counters and tenure and:

- if every criterion is met and the attester is not flagged, ends probation, clears the
  record, and emits `ProbationCompleted { attester, version }`;
- otherwise fails with `ProbationCriteriaNotMet` (or `ProbationFlagged` when flagged) and
  changes nothing. It is idempotent in effect: calling it again after promotion is a
  no-op success.

**Explicit early promotion** is `promote_attester` (global registrar): it ends probation
immediately regardless of criteria, clears any flag, and emits
`ProbationPromoted { early: true }`. The admin may also **extend** probation with
`extend_probation(attester, extra_secs)`, which pushes `ends_at` later and emits
`ProbationExtended { attester, ends_at }`; extension never shortens probation and does
not reset counters.

`flag_probation` and `report_probation_breach` (registrar or guardian) set the flag.
The latter also emits `ProbationBreach { attester, kind }`, which the watchdog treats as
a high-severity alert (`probation_breach`).

## Who reads the status

- **`incentive-pool`** calls `is_payout_eligible` in `approve_work_item` and `claim`
  and returns `PayoutNotEligible` when it is false. A registry without this method leaves
  payouts open.
- **`attestation-registry`** reads `get_probation_limits` on every attestation and
  enforces `rate_cap` per `rate_window_ledgers` with its own window, independent of the
  global `RateLimit`; exceeding it fails with `ProbationRateLimited`.
- **Verifiers** read `get_probation_state` and show a "new attester" notice
  (verification model annotation `attester_on_probation`). It never changes a verdict.

## Interaction with suspend / reinstate / remove / rotate

| Event | Effect on probation |
| --- | --- |
| `suspend_attester*` while probation is running | Flags the attester (`ProbationFlagged`). The clock keeps running but expiry no longer promotes. |
| `suspend_attester*` after probation ended | No effect. |
| `reinstate_attester` | Does **not** clear the flag. The attester must be explicitly promoted. |
| `remove_attester`, `remove_attesters`, `revoke_attester` | Clears the record and counters. Re-enrolling the same key starts a fresh probation. |
| `rotate_attester` | The record and counters move to the successor key unchanged. |
| Re-adding an already enrolled attester | No-op; probation is not restarted. |
| `extend_probation` (admin) | Pushes `ends_at` later; counters unchanged. |

State machine (probation is orthogonal to the allowlist state; `is_attester` is unaffected):

    enroll ──► Probation ──expiry / try_promote (criteria met)──► Standard
                  │  ▲
        flag/suspend │  │ promote_attester (any time)
                  ▼  │
                Flagged ──promote_attester──► Standard

## Events

`ProbationConfigured`, `ProbationPolicyUpdated { version }`,
`ProbationStarted { attester, ends_at }`, `ProbationFlagged`, `ProbationBreach`,
`ProbationExtended { attester, ends_at }`, `ProbationCompleted { attester, version }`,
`ProbationPromoted`. An indexer derives expiry from `ends_at`, since automatic promotion
emits nothing.

## Call graph (acyclic)

Probation data flows in one direction only; no contract calls back into a caller:

    attestation-registry ──record_probation_attestation / record_probation_dispute──► attester-registry
    attester-registry    ──(no outbound calls)──► —
    incentive-pool       ──is_payout_eligible──► attester-registry
    verifiers            ──get_probation_state (read-only)──► attester-registry

`attester-registry` never calls `attestation-registry`, `incentive-pool`, or any verifier,
so there are no circular cross-contract dependencies. Counters are pushed in by the
attestation registry through narrow hooks; the registry only reads them locally in
`try_promote`.
