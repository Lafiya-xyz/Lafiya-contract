# Attester probation

- **Status:** Normative (issue #589)
- **Implementation:** `contracts/attester-registry/src/probation.rs`

Newly enrolled attesters are constrained for a trial period, or until a reviewer promotes
them. Probation is **off by default**: until `set_probation_config` is called with a
non-zero `duration`, no attester is on probation.

## Policy

`set_probation_config(duration, rate_cap, rate_window_ledgers, payout_eligible)` (admin):

| Field | Meaning |
| --- | --- |
| `duration` | Seconds each new enrollment stays on probation; `0` disables probation for new enrollments. |
| `rate_cap` | Maximum attestations per `rate_window_ledgers` while on probation; `0` leaves the rate uncapped. A cap needs a non-zero window. |
| `payout_eligible` | Whether a probationary attester may receive `incentive-pool` payouts. |

Caps are read live from the policy; each attester's end time is fixed at enrollment
(`ends_at = enrolled_at + duration`).

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
auto-promoted. **Explicit early promotion** is `promote_attester` (global registrar): it
ends probation immediately, clears any flag, and emits `ProbationPromoted { early: true }`.

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
| `remove_attester`, `remove_attesters`, `revoke_attester` | Clears the record. Re-enrolling the same key starts a fresh probation. |
| `rotate_attester` | The record moves to the successor key unchanged. |
| Re-adding an already enrolled attester | No-op; probation is not restarted. |

State machine (probation is orthogonal to the allowlist state; `is_attester` is unaffected):

    enroll ──► Probation ──expiry──► Standard
                  │  ▲
        flag/suspend │  │ promote_attester (any time)
                  ▼  │
                Flagged ──promote_attester──► Standard

## Events

`ProbationConfigured`, `ProbationStarted { attester, ends_at }`, `ProbationFlagged`,
`ProbationBreach`, `ProbationPromoted`. An indexer derives expiry from `ends_at`, since
automatic promotion emits nothing.
