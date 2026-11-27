# Testnet Deployment Ledger

Append-only record of every testnet deployment made by CI
(`.github/workflows/deploy-testnet.yml`) and by the testnet-reset automation
(`.github/workflows/testnet-reset.yml`). CI appends rows on the
`testnet-ledger` branch; maintainers merge that branch into `main` periodically.

Flaky runs are tracked by filtering this table for a non-`success` result.

| Time (UTC) | Kind | Commit | attester-registry | attestation-registry | Result | Run |
| --- | --- | --- | --- | --- | --- | --- |

## Incentive-pool testnet rehearsal

The nightly incentive-pool rehearsal (`.github/workflows/incentive-pool-nightly.yml`)
exercises the full payout flow of the incentive-pool contract against the **real
testnet USDC Stellar Asset Contract** (not the Soroban test environment token).
It is the last gate before M2 goes live and is required by
`check_mainnet_readiness.py` (a recent green rehearsal must exist).

### What the rehearsal does

The scripted rehearsal (`scripts/rehearse_incentive_pool.py`, runnable locally or
from CI) performs the following against a fresh testnet deployment:

1. Deploys a **fresh incentive-pool** contract instance to testnet.
2. Funds the pool from a **test donor** account using testnet USDC.
3. Approves and claims payouts across every failure mode below, asserting the
   **typed contract error** and the **accounting state** (pool balance, claim
   status, per-recipient totals) after each step.

### Failure modes covered

| Scenario | Expected behavior |
| --- | --- |
| Happy path | Claim succeeds; pool balance and recipient totals update correctly. |
| No trustline | Recipient has no USDC trustline; claim fails with the typed "no trustline" error; accounting unchanged. |
| Unauthorized trustline | Trustline exists but is not authorized (`AUTH_REQUIRED`); claim fails with the typed authorization error; accounting unchanged. |
| Frozen token | Issuer freezes the recipient's trustline; claim fails with the typed frozen/authorization error; accounting unchanged. |
| Merged account | Recipient account was merged; claim fails with the typed error; accounting unchanged. |
| Paused pool | Pool is paused; claims are rejected with the typed paused error; accounting unchanged. |
| Drift | Accounting drift is detected and surfaced as the typed drift error; state is reported for inspection. |

### Reading the rehearsal report

The rehearsal emits a **machine-readable JSON report** (e.g.
`incentive-pool-rehearsal-report.json`) that is uploaded as a CI artifact on
every run. The report contains, per scenario:

- `scenario` — the failure mode exercised (see table above).
- `status` — `pass` / `fail`.
- `expected_error` / `actual_error` — the typed error codes compared.
- `accounting_before` / `accounting_after` — pool balance, claim status, and
  per-recipient totals captured around the step.
- `tx_hash` — the testnet transaction for manual inspection.

To read a report:

1. Open the workflow run in the **Actions** tab and download the
   `incentive-pool-rehearsal-report` artifact.
2. Confirm every scenario has `status: "pass"` and that `accounting_after`
   matches expectations (unchanged for failure modes, updated for the happy
   path).
3. For any `fail`, use the `tx_hash` to inspect the transaction on a testnet
   explorer and compare `expected_error` against `actual_error`.

A green rehearsal (all scenarios `pass`) is required for the mainnet readiness
checklist; a missing or stale report fails `check_mainnet_readiness.py`.