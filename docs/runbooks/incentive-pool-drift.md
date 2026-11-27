# Runbook: incentive-pool balance drift

**Trigger:** a `BalanceMismatch` event, `claim` failing with
`ReconciliationRequired` (22), or `reconcile()` returning nonzero `drift`.

1. **Measure.** Call `reconcile()` and record `accounted`, `actual_balance`
   and `drift`. Check `is_drift_detected()`.
2. **Pause if negative.** If `drift < 0`, call `pause` while investigating;
   custody is below what the pool owes.
3. **Find the cause.**
   - `BalanceMismatch` on `fund`: the token charges a transfer fee or rebases.
     It is unsupported (ADR-0018); stop funding with it.
   - Negative drift with no mismatch event: check the asset issuer for a
     clawback against the pool address (Horizon operations for the pool).
   - Positive drift: someone transferred tokens directly to the pool.
4. **Resolve.** Once the cause is understood, the admin calls
   `resolve_drift()`. It credits or debits `TotalDeposited` by the drift and
   clears the flag. It fails with `InsufficientPoolBalance` if a debit would
   leave less than has already been paid; top up with `fund` first.
5. **Resume.** `unpause` if paused, confirm `reconcile().drift == 0`, and
   record the incident.

---

## Clawback from the pool or a recipient

Stellar USDC is issued with clawback capability configurable by the issuer. A
clawback against the **pool's** balance makes `token.balance(pool)` lower than
the internal `total_deposited - total_paid`. The existing drift detection then
blocks claims with `ReconciliationRequired` (22). A clawback against a
**recipient** does not change the pool's balance, but it can leave a reserved
work item unpayable when the recipient's account can no longer receive the
funds (or the recipient is frozen).

This section defines the operational policy for absorbing the loss and for
honoring reserved-but-unclaimed work items.

### Detection

- **Pool clawback.** `reconcile()` returns `drift < 0` with no
  `BalanceMismatch` event. Confirm against the issuer's Horizon operations for
  the pool address (a `clawback` operation targeting the pool).
- **Recipient clawback / freeze.** The pool balance is unchanged, but a
  `claim` for a reserved item fails at transfer time. The item remains
  reserved; the recipient cannot be paid until the issuer reverses the
  clawback or the reservation is cancelled by the drift policy below.

### Deterministic drift-resolution policy

When `resolve_drift()` is called with a negative drift (a debit), the pool may
no longer be able to cover every reserved-but-unclaimed work item. The policy
is **deterministic** and applied in this order:

1. **Compute the shortfall.** `shortfall = reserved_total - actual_balance`,
   where `reserved_total` is the sum of amounts for all items in `Reserved`
   state. If `shortfall <= 0`, no reservations are affected; `resolve_drift()`
   proceeds as before.
2. **Prioritize by approval time (FIFO).** Reserved items are ordered by
   `approved_at` ascending, ties broken by `work_id` ascending. This ordering
   is stable and reproducible from on-chain state alone.
3. **Cancel the tail.** Items are kept in FIFO order until `reserved_total`
   fits within `actual_balance`. Every item beyond that point is moved from
   `Reserved` to `Cancelled` and emits a per-item event
   `ReservationCancelled { work_id, amount, reason: Drift }`. The recipient's
   reservation is released; the item may be re-approved later if the pool is
   refunded.
4. **Debit `TotalDeposited`.** After cancellations, `resolve_drift()` debits
   `TotalDeposited` by the drift and clears the flag. It fails with
   `InsufficientPoolBalance` if a debit would leave less than has already been
   paid; top up with `fund` first.

The policy is **cancel-the-tail**, not pro-rata: it is simpler to audit, emits
one event per affected item, and never leaves a partially-funded reservation.
Pro-rata was considered and rejected because it would require mutating every
reserved item's amount and would make the per-item event stream ambiguous.

### Donor communication

- **Who decides.** The pool admin owns the decision to call `resolve_drift()`
  and therefore to cancel reservations. The admin must record the incident
  (cause, drift amount, cancelled `work_id`s) before resuming.
- **Donor notice.** Within one business day of a confirmed pool clawback, the
  admin notifies donors whose funds are affected, stating: the clawback
  amount, which reserved items were cancelled under the FIFO policy, and the
  process for re-approving them once the pool is refunded.
- **Recipient notice.** Recipients of cancelled items are notified via the
  `ReservationCancelled` event; no separate off-chain notice is required, but
  the admin should confirm the event was observed.

### Testing

Clawback scenarios are tested with a real Stellar Asset Contract in the
Soroban test environment (`StellarAssetClient::clawback`), covering:

- **Partial clawback** where reservations exceed the remaining balance: the
  FIFO tail is cancelled and per-item events are emitted.
- **Full clawback** of the pool balance: all reservations are cancelled and
  `resolve_drift()` fails with `InsufficientPoolBalance` until topped up.
- **Clawback during a claim:** a claim in flight when the clawback lands
  fails with `ReconciliationRequired`; after `resolve_drift()` the item is
  either paid (if within the kept FIFO prefix) or cancelled.

---

## ADR-0018 addendum: clawback behavior

**Context.** ADR-0018 declares fee-on-transfer and rebasing tokens unsupported
because they break the `total_deposited - total_paid == balance` invariant.
Clawback is a distinct issuer capability: it is not a transfer fee, but it
also breaks the invariant by reducing `balance` without a corresponding
`total_paid`.

**Decision.** Clawback is treated as an **external loss event**, not as an
unsupported token. The pool detects it via `reconcile()` (negative drift with
no `BalanceMismatch`), blocks claims with `ReconciliationRequired`, and
absorbs the loss through the deterministic FIFO cancel-the-tail policy above.
Recipient-level clawback does not affect the pool balance and is handled at
claim time.

**Consequences.** Reserved items may be cancelled without recipient consent;
this is acceptable because the pool cannot honor them without custody. The
per-item `ReservationCancelled` event makes the decision auditable. Donors
bear the loss pro-rata to their deposits, consistent with the existing
`resolve_drift()` debit semantics.