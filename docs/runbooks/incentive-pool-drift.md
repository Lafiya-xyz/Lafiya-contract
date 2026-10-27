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
