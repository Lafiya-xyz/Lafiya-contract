# Canary attestations

A scheduled workflow (`.github/workflows/canary.yml`, `scripts/canary.sh`)
submits one attestation from a dedicated canary attester and reads it back,
alerting (issue + failed run) on failure or when write+read exceeds
`MAX_LATENCY_SECS` (default 60).

## Hash domain separation

`hash = sha256("lafiya.canary.v1" | network | UTC-hour YYYYMMDDHH)`. The
prefix keeps canary hashes disjoint from real record commitments; the hour
makes each run's hash unique within the hour.

## Canary attester

- Registered once by the admin as a normal attester with a restrictive
  configuration (rate limit set to a few attestations per hour, region
  `canary`); the workflow never holds admin keys.
- Address is public (`CANARY_ATTESTER_ADDRESS` variable). The secret lives only
  in the `testnet` environment secret `CANARY_ATTESTER_SECRET`; rotate it by
  registering a new attester and removing the old one.
- Budget: ~24 submissions/day; fund the account with a month of fees and
  alert on balance below that.

## Exclusions

- Fraud scoring: add the address to `ScoringConfig::excluded_attesters`.
- Dashboard: pass `--exclude-attester <address>` to `dashboard/generate.py`.

## Mainnet

Disabled: `canary.sh` exits without submitting when `NETWORK_NAME=mainnet`
unless `CANARY_MAINNET_APPROVED=true`. Set it only after sign-off.
