# ADR-0018: Incentive-pool work-item lifecycle and custody checks

- **Status:** Proposed
- **Date:** 2026-10-21
- **Deciders:** Lafiya contract maintainers

## Context

`incentive-pool` approvals were claimable forever, could not be retracted,
counters used unchecked `i128` arithmetic, and `fund` trusted the token to
move exactly `amount`. Issues #495, #496, #497 and #498.

## Decision

### Approval expiry (#495)

- `WorkItem` gains `expires_at`. An item is claimable while
  `ledger.timestamp() < expires_at`; at or after it, `claim` fails with
  `WorkItemExpired`.
- **Default TTL: 90 days** (`DEFAULT_APPROVAL_TTL_SECS`), long enough to cover
  a quarterly CHW reporting cycle plus slack. **Max TTL: 365 days**. The admin
  changes both with `set_approval_ttl`; approvers override per item with
  `approve_work_item_with_ttl`, bounded by the max (`InvalidApprovalTtl`).
- `sweep_expired(id)` is permissionless. It tombstones the item
  (`WorkItemSwept`), releases its reservation and emits `WorkItemExpired`.
  Sweeping a swept item is a no-op (`Ok`, no event). Claimed items fail with
  `WorkItemAlreadyClaimed`, cancelled ones with `WorkItemCancelled`.
- **Legacy migration:** a `#[contracttype]` struct cannot gain a field without
  breaking decoding of existing entries, so new items are stored under
  `DataKey::WorkItemV2`. Items approved before the upgrade stay under
  `DataKey::WorkItem` in the old shape and are read with
  `expires_at = approved_at + 90 days`. Legacy items older than 90 days are
  therefore expired immediately on upgrade and can be swept; operators who
  need to honour one should re-check it before upgrading.

### Cancellation (#496)

- `cancel_work_item(caller, id, reason_code)`: `caller` must authorize and be
  the approver or admin (`Unauthorized` otherwise). Soroban has no "either of
  two signers" auth, hence the explicit `caller`.
- Claimed → `WorkItemAlreadyClaimed`; unknown → `WorkItemNotApproved`; double
  cancel → `WorkItemCancelled`; expired → `WorkItemExpired`.
- **Tombstone:** the work-item record is kept and `WorkItemCancelled(id)`
  stores the reason code (`get_cancel_reason`). Re-approving the id fails with
  `WorkItemCancelled`; a fresh id must be used.
- **Race:** transactions in a ledger apply sequentially. Whichever of
  `claim`/`cancel_work_item` is applied first wins; the other fails with
  `WorkItemCancelled` or `WorkItemAlreadyClaimed` respectively.
- Emits `WorkItemCancelled { work_item_id, cancelled_by, reason_code, amount }`.

### Liability accounting

`TotalReserved` (`get_total_reserved`) is the sum of approved, unclaimed, live
items. Approval adds; claim, cancel and sweep subtract. Legacy items were
never added, so releases floor at zero. Gating `withdraw` on the reservation is
left to the separate withdraw/liability issue.

### Checked arithmetic (#497)

Every counter update uses `checked_add`/`checked_sub` and returns
`ArithmeticOverflow` (code 16). The crate denies
`clippy::arithmetic_side_effects` so new unchecked arithmetic fails CI. The
invariant `total_paid <= total_deposited` is enforced by the `claim`/`withdraw`
balance checks and by `resolve_drift`, not by runtime panics.

### Balance-delta custody checks (#498)

- `fund` credits the observed balance delta, not `amount`. A delta that
  differs from `amount` emits `BalanceMismatch` and sets `DriftDetected`.
  A non-positive delta is rejected (`TokenBalanceMismatch`).
- `claim` fails with `ReconciliationRequired` while `DriftDetected` is set or
  the actual balance is below `accounted`, and fails with
  `TokenBalanceMismatch` (reverting) if the transfer moves a different amount.
- `reconcile()` returns `{ accounted, actual_balance, drift }`.
- `resolve_drift()` (admin) adjusts `TotalDeposited` by the drift and clears
  the flag.
- **Supported assets** (extends ADR-0009): plain SEP-41 tokens and Stellar
  assets. Fee-on-transfer and rebasing tokens are unsupported; they are
  detected, not accommodated. For Stellar assets with `AUTH_CLAWBACK_ENABLED`,
  a clawback shows up as negative drift and halts claims until the admin
  resolves it. For `AUTH_REQUIRED` assets, the pool and every attester must
  hold an authorized trustline or transfers fail and roll back.

## Consequences

- Error codes 16–23 are appended; none are renumbered.
- New events: `WorkItemCancelled`, `WorkItemExpired`, `BalanceMismatch`,
  `DriftResolved`.
- `incentive-pool` is not yet registered in `scripts/conformance/contracts.py`,
  so `catalog/*.json` and `@lafiya/events` do not cover it. Registering it
  (snapshot, bindings) and indexer/watchdog handling of the new events are
  follow-ups.
