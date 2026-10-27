# Incentive pool: tiered payout schedule (#510)

Payout amounts are derived from an on-chain schedule keyed by
`(work_type: u32, region_class: u32)` so approvers cannot choose arbitrary
amounts. `region_class` follows the programme's classification of
`AttesterInfo.region` in `attester-registry` (e.g. `0` urban, `1` peri-urban,
`2` rural); work-type codes are programme-defined.

## Changes are timelocked and evented

- `propose_payout_rate(work_type, region_class, amount)` — admin auth; stores a
  pending change with `eta = now + SCHEDULE_TIMELOCK_SECS` (2 days); emits
  `PayoutRateProposed`. Re-proposing replaces the pending change.
- `apply_payout_rate(work_type, region_class)` — permissionless once `eta` has
  passed; bumps the global schedule version and emits `PayoutRateApplied`.
- Version history is reconstructed from `PayoutRateApplied` events; the
  current entry (`get_payout_rate`) carries the version it was applied at.

## Approvals

- `approve_scheduled_work_item(id, attester, work_type, region_class)` —
  approver auth; amount computed from the schedule; records the version
  (`get_work_item_schedule_version`).
- `approve_work_item(id, attester, amount)` — override path for exceptional
  cases; now requires approver **and** admin auth; records version `0`.

## Migration plan for already-approved items

- `WorkItem` is unchanged, so existing approvals decode and remain claimable
  at their originally approved amount.
- The schedule version lives in a separate key; legacy items have none and
  report version `0` ("pre-schedule / override").
- Rollout: upgrade the contract, propose rates for every active
  `(work_type, region_class)`, wait out the timelock, apply, then switch the
  approver tooling to `approve_scheduled_work_item`. Callers of
  `approve_work_item` must add the admin signature from the upgrade onward.
